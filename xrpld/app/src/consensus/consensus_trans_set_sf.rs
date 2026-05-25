//! Bounded `xrpld/app/ledger/ConsensusTransSetSF.*` port.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::blob::Blob;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{CacheClock, MonotonicClock, TaggedCache};
use ledger::TransactionAcquireFilterFactory;
use protocol::{STTx, SerialIter, serialize_prefixed_blob, sha512_half};
use shamap::fetch::SHAMapSyncFilter;
use shamap::tree_node::SHAMapNodeType;
use time::Duration;

use crate::{SharedTransaction, TransactionMaster};

pub type ConsensusTransSetNodeCache<C = MonotonicClock> = TaggedCache<SHAMapHash, Blob, C>;

pub trait ConsensusTransSetApp {
    fn submit_transaction(&self, tx: Arc<STTx>);
    fn fetch_from_cache(&self, hash: &Uint256) -> Option<SharedTransaction>;
}

pub trait ConsensusTransSetSubmitSink {
    fn submit_transaction(&self, tx: Arc<STTx>);
}

pub struct TransactionMasterApp<'a, S, C = MonotonicClock>
where
    S: ConsensusTransSetSubmitSink,
    C: CacheClock,
{
    master: &'a TransactionMaster<C>,
    submit_sink: &'a S,
}

impl<'a, S, C> TransactionMasterApp<'a, S, C>
where
    S: ConsensusTransSetSubmitSink,
    C: CacheClock,
{
    pub fn new(master: &'a TransactionMaster<C>, submit_sink: &'a S) -> Self {
        Self {
            master,
            submit_sink,
        }
    }
}

impl<'a, S, C> ConsensusTransSetApp for TransactionMasterApp<'a, S, C>
where
    S: ConsensusTransSetSubmitSink,
    C: CacheClock,
{
    fn submit_transaction(&self, tx: Arc<STTx>) {
        self.submit_sink.submit_transaction(tx);
    }

    fn fetch_from_cache(&self, hash: &Uint256) -> Option<SharedTransaction> {
        self.master.fetch_from_cache(hash)
    }
}

pub struct ConsensusTransSetSF<'a> {
    insert_node: Box<dyn FnMut(SHAMapHash, Blob) + 'a>,
    get_cached_node: Box<dyn FnMut(SHAMapHash) -> Option<Blob> + 'a>,
    decode_transaction: Box<dyn FnMut(SHAMapHash, &[u8]) -> Result<Arc<STTx>, String> + 'a>,
    fetch_cached_transaction: Box<dyn FnMut(Uint256) -> Option<Arc<STTx>> + 'a>,
    schedule_submit: Box<dyn FnMut(Arc<STTx>) + 'a>,
}

impl<'a> ConsensusTransSetSF<'a> {
    pub fn new<IN, GN, DT, FT, SS>(
        insert_node: IN,
        get_cached_node: GN,
        decode_transaction: DT,
        fetch_cached_transaction: FT,
        schedule_submit: SS,
    ) -> Self
    where
        IN: FnMut(SHAMapHash, Blob) + 'a,
        GN: FnMut(SHAMapHash) -> Option<Blob> + 'a,
        DT: FnMut(SHAMapHash, &[u8]) -> Result<Arc<STTx>, String> + 'a,
        FT: FnMut(Uint256) -> Option<Arc<STTx>> + 'a,
        SS: FnMut(Arc<STTx>) + 'a,
    {
        Self {
            insert_node: Box::new(insert_node),
            get_cached_node: Box::new(get_cached_node),
            decode_transaction: Box::new(decode_transaction),
            fetch_cached_transaction: Box::new(fetch_cached_transaction),
            schedule_submit: Box::new(schedule_submit),
        }
    }

    pub fn from_app<A, C>(app: &'a A, node_cache: &'a ConsensusTransSetNodeCache<C>) -> Self
    where
        A: ConsensusTransSetApp + 'a,
        C: CacheClock + 'a,
    {
        Self::new(
            move |hash, blob| {
                node_cache.insert(hash, blob);
            },
            move |hash| node_cache.retrieve(&hash),
            move |node_hash, payload| decode_consensus_trans_set_transaction(node_hash, payload),
            move |hash| {
                let shared = app.fetch_from_cache(&hash)?;
                let transaction = shared
                    .lock()
                    .expect("transaction mutex must not be poisoned");
                Some(Arc::clone(transaction.get_s_transaction()))
            },
            move |tx| app.submit_transaction(tx),
        )
    }

    pub fn from_shared_app<A, C>(
        app: Arc<A>,
        node_cache: Arc<ConsensusTransSetNodeCache<C>>,
    ) -> ConsensusTransSetSF<'static>
    where
        A: ConsensusTransSetApp + Send + Sync + 'static,
        C: CacheClock + Send + Sync + 'static,
    {
        ConsensusTransSetSF::<'static>::new(
            {
                let node_cache = Arc::clone(&node_cache);
                move |hash, blob| {
                    node_cache.insert(hash, blob);
                }
            },
            {
                let node_cache = Arc::clone(&node_cache);
                move |hash| node_cache.retrieve(&hash)
            },
            move |node_hash, payload| decode_consensus_trans_set_transaction(node_hash, payload),
            {
                let app = Arc::clone(&app);
                move |hash| {
                    let shared = app.fetch_from_cache(&hash)?;
                    let transaction = shared
                        .lock()
                        .expect("transaction mutex must not be poisoned");
                    Some(Arc::clone(transaction.get_s_transaction()))
                }
            },
            move |tx| app.submit_transaction(tx),
        )
    }

    pub fn new_cache() -> ConsensusTransSetNodeCache<MonotonicClock> {
        TaggedCache::new(
            "ConsensusTransSetNodeCache",
            65_536,
            Duration::minutes(30),
            MonotonicClock::default(),
        )
    }
}

pub struct ConsensusTransSetFilterFactory<A, C = MonotonicClock>
where
    A: ConsensusTransSetApp + Send + Sync + 'static,
    C: CacheClock + Send + Sync + 'static,
{
    app: Arc<A>,
    node_cache: Arc<ConsensusTransSetNodeCache<C>>,
}

impl<A, C> ConsensusTransSetFilterFactory<A, C>
where
    A: ConsensusTransSetApp + Send + Sync + 'static,
    C: CacheClock + Send + Sync + 'static,
{
    pub fn new(app: Arc<A>, node_cache: Arc<ConsensusTransSetNodeCache<C>>) -> Self {
        Self { app, node_cache }
    }
}

impl<A, C> TransactionAcquireFilterFactory for ConsensusTransSetFilterFactory<A, C>
where
    A: ConsensusTransSetApp + Send + Sync + 'static,
    C: CacheClock + Send + Sync + 'static,
{
    fn build_filter(&self) -> Box<dyn SHAMapSyncFilter> {
        Box::new(ConsensusTransSetSF::from_shared_app(
            Arc::clone(&self.app),
            Arc::clone(&self.node_cache),
        ))
    }
}

impl SHAMapSyncFilter for ConsensusTransSetSF<'_> {
    fn got_node(
        &mut self,
        from_filter: bool,
        node_hash: SHAMapHash,
        _ledger_seq: u32,
        node_data: Blob,
        node_type: SHAMapNodeType,
    ) {
        if from_filter {
            return;
        }

        (self.insert_node)(node_hash, node_data.clone());

        if node_type != SHAMapNodeType::TransactionNm || node_data.len() <= 16 {
            return;
        }

        if let Ok(tx) = (self.decode_transaction)(node_hash, &node_data[4..]) {
            (self.schedule_submit)(tx);
        }
    }

    fn get_node(&mut self, node_hash: SHAMapHash) -> Option<Blob> {
        if let Some(node_data) = (self.get_cached_node)(node_hash) {
            return Some(node_data);
        }

        let transaction = (self.fetch_cached_transaction)(*node_hash.as_uint256())?;
        Some(encode_consensus_trans_set_transaction_node(
            node_hash,
            transaction.as_ref(),
        ))
    }
}

pub fn decode_consensus_trans_set_transaction(
    node_hash: SHAMapHash,
    node_payload: &[u8],
) -> Result<Arc<STTx>, String> {
    let tx = catch_unwind(AssertUnwindSafe(|| {
        let mut serial = SerialIter::new(node_payload);
        Arc::new(STTx::from_serial_iter(&mut serial))
    }))
    .map_err(|payload| {
        unwind_message(payload).unwrap_or_else(|| "failed to decode consensus transaction".into())
    })?;

    assert_eq!(
        tx.get_transaction_id(),
        *node_hash.as_uint256(),
        "xrpl::ConsensusTransSetSF::gotNode : transaction hash match"
    );

    Ok(tx)
}

pub fn encode_consensus_trans_set_transaction_node(
    node_hash: SHAMapHash,
    transaction: &STTx,
) -> Blob {
    let encoded = serialize_prefixed_blob(protocol::HashPrefix::TransactionId, transaction);
    assert_eq!(
        sha512_half(&encoded),
        *node_hash.as_uint256(),
        "xrpl::ConsensusTransSetSF::getNode : transaction hash match"
    );
    encoded
}

fn unwind_message(payload: Box<dyn std::any::Any + Send>) -> Option<String> {
    match payload.downcast::<String>() {
        Ok(message) => Some(*message),
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => Some((*message).to_string()),
            Err(_) => None,
        },
    }
}
