//! RPC test environment — executes transactions and queries results via RPC handlers.
//! Uses ApplicationRoot in standalone mode with proper ledger state setup.

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{Ledger, LedgerHeader, LEDGER_DEFAULT_TIME_RESOLUTION};
use protocol::{
    account_keylet, calc_account_id, derive_public_key, get_field_by_symbol, AccountID, JsonValue,
    KeyType, LedgerEntryType, Rules, STAmount, STLedgerEntry, STTx, SecretKey,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

use app::{AppOpenLedgerView, ApplicationRoot, ApplicationRootOptions, Transaction};

/// A test account with signing keys.
#[derive(Clone, Debug)]
pub struct TestAccount {
    pub name: String,
    pub id: AccountID,
    pub secret: SecretKey,
    pub public: protocol::PublicKey,
    pub seq: u32,
}

impl TestAccount {
    pub fn new(name: &str) -> Self {
        let seed_bytes: Vec<u8> = name
            .bytes()
            .chain(std::iter::repeat(0u8))
            .take(32)
            .collect();
        let secret = SecretKey::from_bytes(seed_bytes.try_into().unwrap());
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let id = calc_account_id(public.as_bytes());
        Self {
            name: name.to_owned(),
            id,
            secret,
            public,
            seq: 1,
        }
    }

    pub fn id_160(&self) -> Uint160 {
        Uint160::from_slice(self.id.data()).expect("account width")
    }

    pub fn next_seq(&mut self) -> u32 {
        let s = self.seq;
        self.seq += 1;
        s
    }
}

/// Full RPC integration test environment using ApplicationRoot standalone mode.
pub struct RpcTestEnv {
    pub app: ApplicationRoot,
}

impl RpcTestEnv {
    /// Create a new standalone environment with funded accounts.
    pub fn new(funded_accounts: &[(&TestAccount, u64)]) -> Self {
        Self::with_flags_and_entries(funded_accounts, &[], &[], &[])
    }

    /// Create environment with some accounts having specific flags (e.g. DefaultRipple for gateways).
    pub fn with_flags(
        funded_accounts: &[(&TestAccount, u64)],
        flagged: &[(&TestAccount, u32)],
    ) -> Self {
        Self::with_flags_and_entries(funded_accounts, flagged, &[], &[])
    }

    /// Create an environment with additional typed state entries and explicit amendment features.
    pub fn with_entries_and_features(
        funded_accounts: &[(&TestAccount, u64)],
        entries: &[STLedgerEntry],
        features: &[Uint256],
    ) -> Self {
        Self::with_flags_and_entries(funded_accounts, &[], entries, features)
    }

    fn with_flags_and_entries(
        funded_accounts: &[(&TestAccount, u64)],
        flagged: &[(&TestAccount, u32)],
        entries: &[STLedgerEntry],
        features: &[Uint256],
    ) -> Self {
        let mut state_tree = MutableTree::new(1);

        for (account, balance_drops) in funded_accounts {
            let keylet = account_keylet(account.id_160());
            let mut entry =
                STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
            entry.set_account_id(get_field_by_symbol("sfAccount"), account.id);
            entry.set_field_u32(get_field_by_symbol("sfSequence"), account.seq);
            entry.set_field_amount(
                get_field_by_symbol("sfBalance"),
                STAmount::new_native(*balance_drops, false),
            );
            entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
            let flags = flagged
                .iter()
                .find(|(a, _)| a.id == account.id)
                .map(|(_, f)| *f)
                .unwrap_or(0);
            entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
            state_tree
                .add_item(
                    SHAMapNodeType::AccountState,
                    SHAMapItem::new(keylet.key, entry.get_serializer().data().to_vec()),
                )
                .expect("account should insert");
        }

        for entry in entries {
            state_tree
                .add_item(
                    SHAMapNodeType::AccountState,
                    SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
                )
                .expect("typed ledger entry should insert");
        }

        let mut parent = Ledger::from_maps(
            LedgerHeader {
                seq: 1,
                close_time: 1000,
                close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
                ..LedgerHeader::default()
            },
            SyncTree::from_root_with_type(
                state_tree.root(),
                SHAMapType::State,
                false,
                1,
                SyncState::Modifying,
            ),
            SyncTree::from_root_with_type(
                MutableTree::new(1).root(),
                SHAMapType::Transaction,
                false,
                1,
                SyncState::Modifying,
            ),
        );
        parent.set_accepted(1000, LEDGER_DEFAULT_TIME_RESOLUTION, true);
        parent.set_rules(Rules::new(features.iter().copied()));

        let mut app = ApplicationRoot::with_options(ApplicationRootOptions {
            standalone: true,
            ..ApplicationRootOptions::default()
        })
        .expect("standalone root should build");
        let _ = app.attach_default_network_ops_runtime();
        app.on_closed_ledger(Arc::new(parent));
        let _ = app.open_ledger().modify(|view| {
            *view = AppOpenLedgerView::new(2, 10);
            true
        });

        Self { app }
    }

    /// Submit a signed transaction and accept the ledger.
    pub fn submit_and_close(&self, tx: &STTx) {
        let tx = Arc::new(tx.clone());
        let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
        self.app.canonicalize_transaction(&mut cached);
        self.app
            .add_held_transaction(&Transaction::new(Arc::clone(&tx)));
        self.app
            .accept_standalone_ledger()
            .expect("standalone accept should succeed");
    }

    /// Submit multiple transactions then close once.
    pub fn submit_all_and_close(&self, txs: &[&STTx]) {
        for tx in txs {
            let tx = Arc::new((*tx).clone());
            let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
            self.app.canonicalize_transaction(&mut cached);
            self.app
                .add_held_transaction(&Transaction::new(Arc::clone(&tx)));
        }
        self.app
            .accept_standalone_ledger()
            .expect("standalone accept should succeed");
    }

    /// Get ApplicationServerInfo for RPC queries.
    pub fn rpc_source(&self) -> rpc::ApplicationServerInfo<&ApplicationRoot> {
        rpc::ApplicationServerInfo::new(&self.app)
    }
}

/// Helper to create a signed transaction.
pub fn sign_tx(tx: &mut STTx, account: &TestAccount) {
    tx.set_field_vl(
        get_field_by_symbol("sfSigningPubKey"),
        account.public.as_bytes(),
    );
    tx.sign(&account.public, &account.secret, None)
        .expect("signature should succeed");
}

/// Helper JSON object builder.
pub fn json(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<BTreeMap<_, _>>(),
    )
}
