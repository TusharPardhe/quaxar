//! Standalone `ledger_accept` keeps the LedgerHashes skip list current, so
//! transactions submitted after empty closes still apply.

use protocol::{get_field_by_symbol, skip_keylet, STAmount, STTx, TxType};
use rpc_integration_tests::env::*;

fn skip_list_last_seq(env: &RpcTestEnv) -> (u32, u32) {
    let ledger = env.app.closed_ledger().expect("closed ledger");
    let seq = ledger.header().seq;
    let hashes = ledger
        .read(skip_keylet())
        .expect("readable state")
        .expect("LedgerHashes entry");
    (
        seq,
        hashes.get_field_u32(get_field_by_symbol("sfLastLedgerSequence")),
    )
}

#[test]
fn empty_standalone_closes_update_the_skip_list() {
    let mut alice = TestAccount::new("skip_alice");
    let bob = TestAccount::new("skip_bob");
    let env = RpcTestEnv::new(&[(&alice, 1_000_000_000), (&bob, 1_000_000_000)]);

    for _ in 0..3 {
        env.app
            .accept_standalone_ledger()
            .expect("empty close succeeds");
        let (seq, last) = skip_list_last_seq(&env);
        assert_eq!(last, seq - 1, "skip list must end at the parent of {seq}");
    }

    let mut payment = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), alice.id);
        tx.set_account_id(get_field_by_symbol("sfDestination"), bob.id);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), alice.next_seq());
    });
    sign_tx(&mut payment, &alice);
    env.submit_and_close(&payment);
    let (seq, last) = skip_list_last_seq(&env);
    assert_eq!(last, seq - 1);
}
