//! RCL-specific validation types. Ported from `RCLValidations.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use consensus::model::TrieLedger;
use consensus::rcl_support::{ValidationT, ValidationsLedger};
use ledger::{Ledger, LedgerJournal, NullLedgerJournal};
use protocol::{NodeID, PublicKey, STValidation, get_field_by_symbol};

#[derive(Clone)]
pub struct RclValidation {
    val: Arc<STValidation>,
}

impl RclValidation {
    pub fn new(val: Arc<STValidation>) -> Self {
        Self { val }
    }

    pub fn unwrap_arc(&self) -> Arc<STValidation> {
        Arc::clone(&self.val)
    }

    fn cookie(&self) -> u64 {
        u64::from(self.val.get_field_u32(get_field_by_symbol("sfCookie")))
    }

    fn load_fee(&self) -> Option<u32> {
        let field = get_field_by_symbol("sfLoadFee");
        self.val.is_field_present(field).then(|| self.val.get_field_u32(field))
    }
}

impl ValidationT for RclValidation {
    type LedgerId = Uint256;
    type Seq = u32;
    type NodeId = NodeID;
    type NodeKey = PublicKey;
    type Wrapped = Arc<STValidation>;

    fn ledger_id(&self) -> Uint256 {
        self.val.get_ledger_hash()
    }

    fn seq(&self) -> u32 {
        self.val.get_field_u32(get_field_by_symbol("sfLedgerSequence"))
    }

    fn sign_time(&self) -> NetClockTimePoint {
        NetClockTimePoint::new(self.val.get_sign_time())
    }

    fn seen_time(&self) -> NetClockTimePoint {
        NetClockTimePoint::new(self.val.get_seen_time())
    }

    fn key(&self) -> PublicKey {
        *self.val.get_signer_public()
    }

    fn trusted(&self) -> bool {
        self.val.is_trusted()
    }

    fn set_trusted(&mut self) {
        Arc::make_mut(&mut self.val).set_trusted();
    }

    fn set_untrusted(&mut self) {
        Arc::make_mut(&mut self.val).set_untrusted();
    }

    fn full(&self) -> bool {
        self.val.is_full()
    }

    fn node_id(&self) -> NodeID {
        self.val.get_node_id()
    }

    fn load_fee(&self) -> Option<u32> {
        RclValidation::load_fee(self)
    }

    fn cookie(&self) -> u64 {
        RclValidation::cookie(self)
    }

    fn unwrap(self) -> Arc<STValidation> {
        self.val
    }
}

const MAX_ANCESTORS_TRACKED: u32 = 256;

#[derive(Clone)]
pub struct RclValidatedLedger {
    ledger_id: Uint256,
    ledger_seq: u32,
    ancestors: Arc<Vec<Uint256>>,
}

impl RclValidatedLedger {
    pub fn genesis() -> Self {
        Self { ledger_id: Uint256::zero(), ledger_seq: 0, ancestors: Arc::new(vec![Uint256::zero()]) }
    }

    pub fn from_ledger(ledger: &Ledger) -> Self {
        Self::from_ledger_with_journal(ledger, &NullLedgerJournal)
    }

    pub fn from_ledger_with_journal<J: LedgerJournal>(ledger: &Ledger, journal: &J) -> Self {
        let header = ledger.header();
        let ledger_seq = header.seq;
        let ledger_id = *header.hash.as_uint256();

        let min_seq = ledger_seq.saturating_sub(MAX_ANCESTORS_TRACKED.min(ledger_seq));
        let mut ancestors = Vec::with_capacity((ledger_seq - min_seq + 1) as usize);
        for seq in min_seq..=ledger_seq {
            let hash = ledger.hash_of_seq(seq, journal).map(|h| *h.as_uint256()).unwrap_or_else(Uint256::zero);
            ancestors.push(hash);
        }

        Self { ledger_id, ledger_seq, ancestors: Arc::new(ancestors) }
    }

    fn min_seq(&self) -> u32 {
        self.ledger_seq + 1 - self.ancestors.len() as u32
    }
}

impl TrieLedger for RclValidatedLedger {
    type Seq = u32;
    type Id = Uint256;

    fn genesis() -> Self {
        RclValidatedLedger::genesis()
    }

    fn seq(&self) -> u32 {
        self.ledger_seq
    }

    fn ancestor(&self, s: u32) -> Uint256 {
        if s > self.ledger_seq {
            return Uint256::zero();
        }
        if s == self.ledger_seq {
            return self.ledger_id;
        }
        let min_seq = self.min_seq();
        if s < min_seq {
            return Uint256::zero();
        }
        self.ancestors[(s - min_seq) as usize]
    }

    fn mismatch(&self, other: &Self) -> u32 {
        let max_check = self.ledger_seq.min(other.ledger_seq) + 1;
        let mut lo = 0u32;
        let mut hi = max_check;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ancestor(mid) == other.ancestor(mid) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

impl ValidationsLedger for RclValidatedLedger {
    fn id(&self) -> Uint256 {
        self.ledger_id
    }
}

pub struct RclValidationsAdaptor {
    ledgers: parking_lot::Mutex<std::collections::HashMap<Uint256, RclValidatedLedger>>,
    now: Arc<dyn Fn() -> NetClockTimePoint + Send + Sync>,
}

impl RclValidationsAdaptor {
    /// Construct an adaptor with the given network-time source. The caller
    /// supplies this because `Validations` (Phase 5) has no way to derive
    /// "now" itself: rippled's `Application`-backed `RCLValidationsAdaptor`
    /// reads the same clock the rest of the node's networking layer uses,
    /// which this crate does not own.
    pub fn new(now: impl Fn() -> NetClockTimePoint + Send + Sync + 'static) -> Self {
        Self { ledgers: parking_lot::Mutex::new(std::collections::HashMap::new()), now: Arc::new(now) }
    }

    pub fn register_ledger(&self, ledger: &Ledger) {
        let wrapped = RclValidatedLedger::from_ledger(ledger);
        self.ledgers.lock().insert(wrapped.id(), wrapped);
    }
}

impl consensus::rcl_support::ValidationsAdaptor for RclValidationsAdaptor {
    type Ledger = RclValidatedLedger;
    type Validation = RclValidation;

    fn now(&self) -> NetClockTimePoint {
        (self.now)()
    }

    fn acquire(&self, ledger_id: &Uint256) -> Option<RclValidatedLedger> {
        self.ledgers.lock().get(ledger_id).cloned()
    }
}

impl consensus::rcl::AsValidationKey<RclValidationsAdaptor> for Arc<STValidation> {
    fn node_key(&self) -> PublicKey {
        *self.get_signer_public()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger::Ledger as LedgerImpl;
    use protocol::{KeyType, SecretKey, calc_node_id, derive_public_key, generate_secret_key, random_seed};

    fn signed_validation(ledger_hash: Uint256, seq: u32, sign_time: u32) -> Arc<STValidation> {
        let seed = random_seed();
        let secret_key: SecretKey = generate_secret_key(KeyType::Secp256k1, &seed).expect("secret key generation should succeed");
        let public_key = derive_public_key(KeyType::Secp256k1, &secret_key).expect("public key derivation should succeed");
        let node_id = calc_node_id(&public_key);

        let val = STValidation::new_signed(sign_time, &public_key, node_id, &secret_key, |v| {
            v.set_field_h256(get_field_by_symbol("sfLedgerHash"), ledger_hash);
            v.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
        })
        .expect("validation signing should succeed");
        Arc::new(val)
    }

    #[test]
    fn rcl_validation_exposes_ledger_hash_and_seq() {
        let hash = Uint256::from_slice(&[7; 32]).unwrap();
        let val = RclValidation::new(signed_validation(hash, 42, 1000));

        assert_eq!(val.ledger_id(), hash);
        assert_eq!(ValidationT::seq(&val), 42);
        assert!(val.trusted());
    }

    #[test]
    fn rcl_validation_cookie_defaults_to_zero() {
        let hash = Uint256::from_slice(&[7; 32]).unwrap();
        let val = RclValidation::new(signed_validation(hash, 1, 1000));
        assert_eq!(ValidationT::cookie(&val), 0);
    }

    #[test]
    fn rcl_validated_ledger_ancestor_lookups_match_genesis_and_self() {
        let ledger = LedgerImpl::from_ledger_seq_and_close_time(5, 500, false);
        let wrapped = RclValidatedLedger::from_ledger(&ledger);

        assert_eq!(TrieLedger::seq(&wrapped), 5);
        assert_eq!(wrapped.ancestor(5), wrapped.ledger_id);
    }

    #[test]
    fn rcl_validations_adaptor_acquires_registered_ledgers() {
        let adaptor = RclValidationsAdaptor::new(|| NetClockTimePoint::new(1000));
        let ledger = LedgerImpl::from_ledger_seq_and_close_time(3, 300, false);
        adaptor.register_ledger(&ledger);

        let id = *ledger.header().hash.as_uint256();
        assert!(consensus::rcl_support::ValidationsAdaptor::acquire(&adaptor, &id).is_some());
    }
}
