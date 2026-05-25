//! the reference implementation compatibility surface.
//!
//! This ports the exact current deterministic `preflight(...)`,
//! `preclaim(...)`, and `doApply()` shells.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCreatePreflightFacts {
    pub tx_account_is_destination: bool,
    pub send_max_is_legal: bool,
    pub send_max_signum_positive: bool,
    pub send_max_currency_is_bad: bool,
    pub expiration: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckCreatePreclaimFacts {
    pub destination_exists: bool,
    pub destination_disallow_incoming_check: bool,
    pub destination_is_pseudo_account: bool,
    pub destination_require_dest_tag: bool,
    pub tx_has_destination_tag: bool,
    pub send_max_is_native: bool,
    pub send_max_issuer_is_source: bool,
    pub send_max_issuer_is_destination: bool,
    pub send_max_issuer_globally_frozen: bool,
    pub source_to_issuer_trustline_frozen: bool,
    pub issuer_to_destination_trustline_frozen: bool,
    pub tx_expired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckCreateApplyFacts {
    pub source_account: String,
    pub destination_account: String,
    pub sequence: u32,
    pub destination_equals_source: bool,
    pub send_max: String,
    pub source_tag: Option<u32>,
    pub destination_tag: Option<u32>,
    pub invoice_id: Option<[u8; 32]>,
    pub expiration: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckCreateMutation {
    pub source_account: String,
    pub destination_account: String,
    pub sequence: u32,
    pub send_max: String,
    pub source_tag: Option<u32>,
    pub destination_tag: Option<u32>,
    pub invoice_id: Option<[u8; 32]>,
    pub expiration: Option<u32>,
    pub destination_node: Option<u64>,
    pub owner_node: u64,
}

pub trait CheckCreateApplySink {
    fn source_account_exists(&mut self) -> bool;
    fn reserve_sufficient(&mut self) -> bool;
    fn insert_destination_dir(&mut self) -> Option<u64>;
    fn insert_owner_dir(&mut self) -> Option<u64>;
    fn create_check(&mut self, mutation: CheckCreateMutation);
    fn adjust_owner_count(&mut self, delta: i32);
}

pub fn run_check_create_preflight(facts: CheckCreatePreflightFacts) -> NotTec {
    if facts.tx_account_is_destination {
        return Ter::TEM_REDUNDANT;
    }

    if !facts.send_max_is_legal || !facts.send_max_signum_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.send_max_currency_is_bad {
        return Ter::TEM_BAD_CURRENCY;
    }

    if facts.expiration == Some(0) {
        return Ter::TEM_BAD_EXPIRATION;
    }

    Ter::TES_SUCCESS
}

pub fn run_check_create_preclaim(facts: CheckCreatePreclaimFacts) -> Ter {
    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    if facts.destination_disallow_incoming_check || facts.destination_is_pseudo_account {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.destination_require_dest_tag && !facts.tx_has_destination_tag {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    if !facts.send_max_is_native {
        if facts.send_max_issuer_globally_frozen {
            return Ter::TEC_FROZEN;
        }

        if !facts.send_max_issuer_is_source && facts.source_to_issuer_trustline_frozen {
            return Ter::TEC_FROZEN;
        }

        if !facts.send_max_issuer_is_destination && facts.issuer_to_destination_trustline_frozen {
            return Ter::TEC_FROZEN;
        }
    }

    if facts.tx_expired {
        return Ter::TEC_EXPIRED;
    }

    Ter::TES_SUCCESS
}

pub fn run_check_create_do_apply<S: CheckCreateApplySink>(
    facts: CheckCreateApplyFacts,
    sink: &mut S,
) -> Ter {
    if !sink.source_account_exists() {
        return Ter::TEF_INTERNAL;
    }

    if !sink.reserve_sufficient() {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let destination_node = if facts.destination_equals_source {
        None
    } else {
        match sink.insert_destination_dir() {
            Some(page) => Some(page),
            None => return Ter::TEC_DIR_FULL,
        }
    };

    let owner_node = match sink.insert_owner_dir() {
        Some(page) => page,
        None => return Ter::TEC_DIR_FULL,
    };

    sink.create_check(CheckCreateMutation {
        source_account: facts.source_account,
        destination_account: facts.destination_account,
        sequence: facts.sequence,
        send_max: facts.send_max,
        source_tag: facts.source_tag,
        destination_tag: facts.destination_tag,
        invoice_id: facts.invoice_id,
        expiration: facts.expiration,
        destination_node,
        owner_node,
    });
    sink.adjust_owner_count(1);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        CheckCreateApplyFacts, CheckCreateApplySink, CheckCreateMutation, CheckCreatePreclaimFacts,
        CheckCreatePreflightFacts, run_check_create_do_apply, run_check_create_preclaim,
        run_check_create_preflight,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestApplySink {
        source_account_exists: bool,
        reserve_sufficient: bool,
        destination_dir_page: Option<u64>,
        owner_dir_page: Option<u64>,
        created: Option<CheckCreateMutation>,
        owner_count_deltas: Vec<i32>,
        events: Vec<String>,
    }

    impl TestApplySink {
        fn new() -> Self {
            Self {
                source_account_exists: true,
                reserve_sufficient: true,
                destination_dir_page: Some(11),
                owner_dir_page: Some(22),
                created: None,
                owner_count_deltas: Vec::new(),
                events: Vec::new(),
            }
        }
    }

    impl CheckCreateApplySink for TestApplySink {
        fn source_account_exists(&mut self) -> bool {
            self.events.push("source_exists".to_string());
            self.source_account_exists
        }

        fn reserve_sufficient(&mut self) -> bool {
            self.events.push("reserve".to_string());
            self.reserve_sufficient
        }

        fn insert_destination_dir(&mut self) -> Option<u64> {
            self.events.push("destination_dir".to_string());
            self.destination_dir_page
        }

        fn insert_owner_dir(&mut self) -> Option<u64> {
            self.events.push("owner_dir".to_string());
            self.owner_dir_page
        }

        fn create_check(&mut self, mutation: CheckCreateMutation) {
            self.events.push("create".to_string());
            self.created = Some(mutation);
        }

        fn adjust_owner_count(&mut self, delta: i32) {
            self.events.push(format!("adjust:{delta}"));
            self.owner_count_deltas.push(delta);
        }
    }

    fn preflight_facts() -> CheckCreatePreflightFacts {
        CheckCreatePreflightFacts {
            tx_account_is_destination: false,
            send_max_is_legal: true,
            send_max_signum_positive: true,
            send_max_currency_is_bad: false,
            expiration: None,
        }
    }

    fn preclaim_facts() -> CheckCreatePreclaimFacts {
        CheckCreatePreclaimFacts {
            destination_exists: true,
            destination_disallow_incoming_check: false,
            destination_is_pseudo_account: false,
            destination_require_dest_tag: false,
            tx_has_destination_tag: false,
            send_max_is_native: true,
            send_max_issuer_is_source: false,
            send_max_issuer_is_destination: false,
            send_max_issuer_globally_frozen: false,
            source_to_issuer_trustline_frozen: false,
            issuer_to_destination_trustline_frozen: false,
            tx_expired: false,
        }
    }

    fn apply_facts() -> CheckCreateApplyFacts {
        CheckCreateApplyFacts {
            source_account: "alice".to_string(),
            destination_account: "bob".to_string(),
            sequence: 7,
            destination_equals_source: false,
            send_max: "USD:50".to_string(),
            source_tag: Some(2),
            destination_tag: Some(3),
            invoice_id: Some([4; 32]),
            expiration: Some(9),
        }
    }

    #[test]
    fn check_create_preflight_rejects_check_to_self() {
        let result = run_check_create_preflight(CheckCreatePreflightFacts {
            tx_account_is_destination: true,
            send_max_is_legal: false,
            send_max_signum_positive: false,
            send_max_currency_is_bad: true,
            expiration: Some(0),
        });

        assert_eq!(result, Ter::TEM_REDUNDANT);
        assert_eq!(trans_token(result), "temREDUNDANT");
    }

    #[test]
    fn check_create_preflight_rejects_bad_amount_before_currency() {
        let result = run_check_create_preflight(CheckCreatePreflightFacts {
            send_max_is_legal: false,
            send_max_currency_is_bad: true,
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_BAD_AMOUNT);
        assert_eq!(trans_token(result), "temBAD_AMOUNT");
    }

    #[test]
    fn check_create_preflight_rejects_bad_currency() {
        let result = run_check_create_preflight(CheckCreatePreflightFacts {
            send_max_currency_is_bad: true,
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_BAD_CURRENCY);
        assert_eq!(trans_token(result), "temBAD_CURRENCY");
    }

    #[test]
    fn check_create_preflight_rejects_zero_expiration() {
        let result = run_check_create_preflight(CheckCreatePreflightFacts {
            expiration: Some(0),
            ..preflight_facts()
        });

        assert_eq!(result, Ter::TEM_BAD_EXPIRATION);
        assert_eq!(trans_token(result), "temBAD_EXPIRATION");
    }

    #[test]
    fn check_create_preclaim_rejects_missing_destination() {
        let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
            destination_exists: false,
            destination_disallow_incoming_check: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_NO_DST);
        assert_eq!(trans_token(result), "tecNO_DST");
    }

    #[test]
    fn check_create_preclaim_rejects_disallowed_destination() {
        let disallow = run_check_create_preclaim(CheckCreatePreclaimFacts {
            destination_disallow_incoming_check: true,
            ..preclaim_facts()
        });
        let pseudo = run_check_create_preclaim(CheckCreatePreclaimFacts {
            destination_is_pseudo_account: true,
            ..preclaim_facts()
        });

        assert_eq!(disallow, Ter::TEC_NO_PERMISSION);
        assert_eq!(pseudo, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn check_create_preclaim_rejects_missing_destination_tag() {
        let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
            destination_require_dest_tag: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
        assert_eq!(trans_token(result), "tecDST_TAG_NEEDED");
    }

    #[test]
    fn check_create_preclaim_rejects_frozen_issuer() {
        let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
            send_max_is_native: false,
            send_max_issuer_globally_frozen: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_FROZEN);
        assert_eq!(trans_token(result), "tecFROZEN");
    }

    #[test]
    fn check_create_preclaim_rejects_frozen_source_trustline() {
        let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
            send_max_is_native: false,
            send_max_issuer_is_source: false,
            source_to_issuer_trustline_frozen: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_FROZEN);
    }

    #[test]
    fn check_create_preclaim_rejects_frozen_destination_trustline() {
        let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
            send_max_is_native: false,
            send_max_issuer_is_destination: false,
            issuer_to_destination_trustline_frozen: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_FROZEN);
    }

    #[test]
    fn check_create_preclaim_rejects_expired_check() {
        let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
            tx_expired: true,
            ..preclaim_facts()
        });

        assert_eq!(result, Ter::TEC_EXPIRED);
        assert_eq!(trans_token(result), "tecEXPIRED");
    }

    #[test]
    fn check_create_do_apply_maps_missing_source() {
        let mut sink = TestApplySink::new();
        sink.source_account_exists = false;

        let result = run_check_create_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(trans_token(result), "tefINTERNAL");
        assert_eq!(sink.events, ["source_exists"]);
    }

    #[test]
    fn check_create_do_apply_maps_insufficient_reserve() {
        let mut sink = TestApplySink::new();
        sink.reserve_sufficient = false;

        let result = run_check_create_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(sink.events, ["source_exists", "reserve"]);
    }

    #[test]
    fn check_create_do_apply_maps_destination_dir_failure() {
        let mut sink = TestApplySink::new();
        sink.destination_dir_page = None;

        let result = run_check_create_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEC_DIR_FULL);
        assert_eq!(sink.events, ["source_exists", "reserve", "destination_dir"]);
    }

    #[test]
    fn check_create_do_apply_maps_owner_dir_failure() {
        let mut sink = TestApplySink::new();
        sink.owner_dir_page = None;

        let result = run_check_create_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TEC_DIR_FULL);
        assert_eq!(
            sink.events,
            ["source_exists", "reserve", "destination_dir", "owner_dir"]
        );
    }

    #[test]
    fn check_create_do_apply_preserves_success_shape() {
        let mut sink = TestApplySink::new();

        let result = run_check_create_do_apply(apply_facts(), &mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.events,
            [
                "source_exists",
                "reserve",
                "destination_dir",
                "owner_dir",
                "create",
                "adjust:1"
            ]
        );
        assert_eq!(sink.owner_count_deltas, vec![1]);
        let created = sink.created.expect("created check");
        assert_eq!(created.destination_node, Some(11));
        assert_eq!(created.owner_node, 22);
        assert_eq!(created.source_tag, Some(2));
        assert_eq!(created.destination_tag, Some(3));
        assert_eq!(created.invoice_id, Some([4; 32]));
        assert_eq!(created.expiration, Some(9));
    }
}
