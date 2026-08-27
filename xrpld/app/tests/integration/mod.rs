//! Integration tests ported from C++ test::jtx test files.

mod account_delete;
mod account_features;
mod amm;
mod amm_engine;
mod check;
mod directory_ops;
mod escrow;
mod fixtures;
mod flow_paths;
mod nftoken;
mod nftoken_engine;
mod nftoken_trading;
mod offer;
mod offer_crossing;
mod offer_engine;
mod paychan;
mod payment;
mod payment_engine;
mod pipeline;
mod trust_set;
mod vault;

use basics::base_uint::Uint160;
use ledger::{ApplyView, ReadView};
use protocol::{STTx, Ter, TxType, account_keylet, get_field_by_symbol};

/// Handler-level integration tests bypass the fee shell, so capture the
/// submitting account's balance at the same pre-fee boundary before dispatch.
fn handle_real_dispatch<V: ApplyView>(
    view: &mut V,
    tx: &STTx,
    tx_type: TxType,
    _unused: Option<i64>,
) -> Ter {
    let pre_fee_balance = if tx.is_field_present(get_field_by_symbol("sfAccount")) {
        let account = tx.get_account_id(get_field_by_symbol("sfAccount"));
        view.read(account_keylet(Uint160::from_void(account.data())))
            .ok()
            .flatten()
            .map(|sle| {
                sle.get_field_amount(get_field_by_symbol("sfBalance"))
                    .xrp()
                    .drops()
            })
    } else {
        None
    };
    app::state::transactor_dispatcher::handle_real_dispatch(view, tx, tx_type, pre_fee_balance)
}
