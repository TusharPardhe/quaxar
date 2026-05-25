use ledger::{ApplyView, FlowSandbox, ReadView, flow_sandbox::Action};
use protocol::{LedgerEntryType, Ter, XRPAmount, get_field_by_symbol};

pub fn check_invariants<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    let mut xrp_balance_change: i64 = 0;

    for (index, entry) in sandbox.items() {
        let is_delete = entry.action == Action::Erase;
        let after = if is_delete { None } else { Some(&entry.sle) };
        let before = sandbox
            .peek_parent(protocol::Keylet::new(
                after
                    .map(|a| a.get_type())
                    .unwrap_or_else(|| entry.sle.get_type()),
                *index,
            ))
            .ok()
            .flatten();

        let before_sle = before.as_deref();
        let after_sle = after.map(|s| &**s);

        // 4. LedgerEntryTypesMatch
        if let (Some(b), Some(a)) = (before_sle, after_sle) {
            if b.get_type() != a.get_type() {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }

        // 2. AccountRootsNotDeleted
        if is_delete {
            let sle_to_delete = before_sle.unwrap_or(&*entry.sle);
            if sle_to_delete.get_type() == LedgerEntryType::AccountRoot {
                if txn_type != protocol::TxType::ACCOUNT_DELETE {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
        }

        let sle_type = after_sle
            .map(|s| s.get_type())
            .unwrap_or_else(|| before_sle.unwrap_or(&*entry.sle).get_type());

        match sle_type {
            LedgerEntryType::AccountRoot => {
                // 8. XRPBalanceChecks
                if let Some(a) = after_sle {
                    let balance_field = get_field_by_symbol("sfBalance");
                    if a.is_field_present(balance_field) {
                        let bal = a.get_field_amount(balance_field);
                        if bal.negative() || bal.xrp().drops() > protocol::INITIAL_XRP.drops() {
                            return Ter::TEC_INVARIANT_FAILED;
                        }
                    }
                }

                // 7. ValidNewAccountRoot
                // when DeletableAccounts is enabled (always on testnet/mainnet).
                if entry.action == Action::Insert {
                    if let Some(a) = after_sle {
                        let seq = a.get_field_u32(get_field_by_symbol("sfSequence"));
                        let expected_seq = sandbox.header().seq;
                        if seq != expected_seq && seq != 0 {
                            return Ter::TEC_INVARIANT_FAILED;
                        }
                    }
                }

                // 1. XRPNotCreated (AccountRoot)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Escrow => {
                // 6. NoZeroEscrow
                if let Some(a) = after_sle {
                    let amt = a.get_field_amount(get_field_by_symbol("sfAmount"));
                    if amt.xrp().drops() <= 0 {
                        return Ter::TEC_INVARIANT_FAILED;
                    }
                }

                // 1. XRPNotCreated (Escrow)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::PayChannel => {
                // 1. XRPNotCreated (PayChannel)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Offer => {
                // 5. NoBadOffers
                if let Some(a) = after_sle {
                    let gets = a.get_field_amount(get_field_by_symbol("sfTakerGets"));
                    let pays = a.get_field_amount(get_field_by_symbol("sfTakerPays"));
                    if gets.negative()
                        || gets.mantissa() == 0
                        || pays.negative()
                        || pays.mantissa() == 0
                    {
                        return Ter::TEC_INVARIANT_FAILED;
                    }
                }
            }
            _ => {}
        }
    }

    // 1. XRPNotCreated (finalize)
    // Since our sandbox does not contain the fee deduction (it's applied to the parent view),
    // the net XRP change inside the sandbox MUST be <= 0.
    if xrp_balance_change > 0 {
        return Ter::TEC_INVARIANT_FAILED;
    }

    // 3. TransactionFeeCheck
    if fee.drops() < 0 || fee.drops() > protocol::INITIAL_XRP.drops() {
        return Ter::TEC_INVARIANT_FAILED;
    }

    result
}
