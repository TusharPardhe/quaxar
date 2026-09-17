use basics::base_uint::{Uint192, Uint256};
use protocol::{AccountID, MPTAmount, MPTIssue, STAmount, Ter, get_field_by_symbol};
use tx::{EscrowFinishApplyFacts, EscrowFinishApplySink, run_escrow_finish_do_apply};

#[derive(Default)]
struct Sink {
    transfer_calls: usize,
}

impl EscrowFinishApplySink for Sink {
    fn transfer_escrow_amount(&mut self) -> Ter {
        self.transfer_calls += 1;
        Ter::TES_SUCCESS
    }

    fn remove_escrow_entry(&mut self) {}

    fn adjust_owner_count(&mut self, _account: &AccountID, _delta: i32) {}
}

#[test]
fn escrow_finish_do_apply_defensively_rejects_non_xrp_without_token_escrow() {
    let owner = AccountID::from_array([0x32; 20]);
    let destination = AccountID::from_array([0x33; 20]);
    let amount = STAmount::from_mpt_amount(
        get_field_by_symbol("sfAmount"),
        MPTAmount::from_value(1),
        MPTIssue::new(Uint192::from_array([0x44; 24])),
    );
    let mut sink = Sink::default();

    let result = run_escrow_finish_do_apply(
        EscrowFinishApplyFacts {
            amount,
            token_escrow_enabled: false,
            destination,
            owner,
            escrow_key: Uint256::default(),
            owner_node: 0,
            destination_node: None,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEM_DISABLED);
    assert_eq!(sink.transfer_calls, 0);
}
