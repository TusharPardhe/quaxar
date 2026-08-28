use basics::{
    math::base_uint::Uint160,
    number::{NumberRoundModeGuard, RoundingMode},
};
use ledger::{ApplyView, ReadView, adjust_owner_count, dir_insert, dir_remove};
use protocol::{
    AccountID, Issue, Keylet, PublicKey, STAmount, STArray, STLedgerEntry, STObject, STTx,
    STXChainBridge, Ter, XChainBridgeChainType, XChainClaimAttestation, XChainClaimAttestations,
    XChainCreateAccountAttestation, XChainCreateAccountAttestations, XRPAmount, attestations,
    calc_account_id, get_field_by_symbol as sf, lsfDisableMaster,
};
use std::collections::HashMap;
use std::sync::Arc;

pub fn apply_xchain_create_bridge<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let reward = sttx.get_field_amount(sf("sfSignatureReward"));
    let min_account_create = if sttx.is_field_present(sf("sfMinAccountCreateAmount")) {
        Some(sttx.get_field_amount(sf("sfMinAccountCreateAmount")))
    } else {
        None
    };

    let sle_acct = match view.peek(protocol::account_keylet(Uint160::from_void(account.data()))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };

    let chain_type = STXChainBridge::src_chain(account == bridge_spec.locking_chain_door());
    let bridge_keylet = protocol::bridge_keylet_from_door_issue(
        Uint160::from_void(bridge_spec.door(chain_type).data()),
        *bridge_spec.issue(chain_type).get::<Issue>(),
    );

    let mut sle_bridge = STLedgerEntry::new(bridge_keylet);
    sle_bridge.set_account_id(sf("sfAccount"), account);
    sle_bridge.set_field_amount(sf("sfSignatureReward"), reward);
    if let Some(mac) = min_account_create {
        sle_bridge.set_field_amount(sf("sfMinAccountCreateAmount"), mac);
    }
    sle_bridge.set_field_xchain_bridge(sf("sfXChainBridge"), bridge_spec);
    sle_bridge.set_field_u64(sf("sfXChainClaimID"), 0);
    sle_bridge.set_field_u64(sf("sfXChainAccountCreateCount"), 0);
    sle_bridge.set_field_u64(sf("sfXChainAccountClaimCount"), 0);

    // Add to owner directory
    let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
    let describe = |obj: &mut STObject| {
        obj.set_account_id(sf("sfOwner"), account);
    };
    let page = match dir_insert(
        view as &mut dyn ApplyView,
        &owner_dir,
        bridge_keylet.key,
        &describe,
    ) {
        Ok(Some(p)) => p,
        Ok(None) => return Ter::TEC_DIR_FULL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    sle_bridge.set_field_u64(sf("sfOwnerNode"), page);

    if adjust_owner_count(view, &sle_acct, 1).is_err() || view.insert(Arc::new(sle_bridge)).is_err()
    {
        return Ter::TEF_BAD_LEDGER;
    }

    Ter::TES_SUCCESS
}

pub fn apply_xchain_modify_bridge<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let reward = if sttx.is_field_present(sf("sfSignatureReward")) {
        Some(sttx.get_field_amount(sf("sfSignatureReward")))
    } else {
        None
    };
    let min_account_create = if sttx.is_field_present(sf("sfMinAccountCreateAmount")) {
        Some(sttx.get_field_amount(sf("sfMinAccountCreateAmount")))
    } else {
        None
    };
    let flags = sttx.get_field_u32(sf("sfFlags"));
    let clear_account_create = (flags & 0x0001_0000) != 0;

    let chain_type = STXChainBridge::src_chain(account == bridge_spec.locking_chain_door());
    let bridge_keylet = protocol::bridge_keylet_from_door_issue(
        Uint160::from_void(bridge_spec.door(chain_type).data()),
        *bridge_spec.issue(chain_type).get::<Issue>(),
    );

    let mut sle_bridge = match view.peek(bridge_keylet) {
        Ok(Some(sle)) => (*sle).clone(),
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };

    if let Some(r) = reward {
        sle_bridge.set_field_amount(sf("sfSignatureReward"), r);
    }
    if let Some(mac) = min_account_create {
        sle_bridge.set_field_amount(sf("sfMinAccountCreateAmount"), mac);
    }
    if clear_account_create && sle_bridge.is_field_present(sf("sfMinAccountCreateAmount")) {
        sle_bridge.make_field_absent(sf("sfMinAccountCreateAmount"));
    }

    view.update(Arc::new(sle_bridge))
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

pub fn apply_xchain_create_claim_id<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let reward = sttx.get_field_amount(sf("sfSignatureReward"));
    let other_chain_src = sttx.get_account_id(sf("sfOtherChainSource"));

    let sle_acct = match view.peek(protocol::account_keylet(Uint160::from_void(account.data()))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };

    let sle_bridge = match read_bridge_helper(view, &bridge_spec) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };

    let claim_id = sle_bridge.get_field_u64(sf("sfXChainClaimID")) + 1;
    if claim_id == 0 {
        return Ter::TEC_INTERNAL;
    }

    let mut updated_bridge = (*sle_bridge).clone();
    updated_bridge.set_field_u64(sf("sfXChainClaimID"), claim_id);

    let claim_id_keylet = protocol::xchain_owned_claim_id_keylet_from_bridge(
        Uint160::from_void(bridge_spec.locking_chain_door().data()),
        *bridge_spec.locking_chain_issue().get::<Issue>(),
        Uint160::from_void(bridge_spec.issuing_chain_door().data()),
        *bridge_spec.issuing_chain_issue().get::<Issue>(),
        claim_id,
    );

    match view.exists(claim_id_keylet) {
        Ok(true) => return Ter::TEC_INTERNAL,
        Ok(false) => {}
        Err(_) => return Ter::TEF_BAD_LEDGER,
    }

    let mut sle_claim_id = STLedgerEntry::new(claim_id_keylet);
    sle_claim_id.set_account_id(sf("sfAccount"), account);
    sle_claim_id.set_field_xchain_bridge(sf("sfXChainBridge"), bridge_spec);
    sle_claim_id.set_field_u64(sf("sfXChainClaimID"), claim_id);
    sle_claim_id.set_account_id(sf("sfOtherChainSource"), other_chain_src);
    sle_claim_id.set_field_amount(sf("sfSignatureReward"), reward);
    sle_claim_id.set_field_array(
        sf("sfXChainClaimAttestations"),
        STArray::new(sf("sfXChainClaimAttestations")),
    );

    // Add to owner directory
    let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
    let describe = |obj: &mut STObject| {
        obj.set_account_id(sf("sfOwner"), account);
    };
    let page = match dir_insert(
        view as &mut dyn ApplyView,
        &owner_dir,
        claim_id_keylet.key,
        &describe,
    ) {
        Ok(Some(p)) => p,
        Ok(None) => return Ter::TEC_DIR_FULL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    sle_claim_id.set_field_u64(sf("sfOwnerNode"), page);

    if adjust_owner_count(view, &sle_acct, 1).is_err()
        || view.insert(Arc::new(sle_claim_id)).is_err()
        || view.update(Arc::new(updated_bridge)).is_err()
    {
        return Ter::TEF_BAD_LEDGER;
    }

    Ter::TES_SUCCESS
}

fn read_bridge_helper<V: ReadView>(
    view: &V,
    bridge_spec: &STXChainBridge,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    let try_get = |chain_type: XChainBridgeChainType| -> Result<Option<Arc<STLedgerEntry>>, Ter> {
        let bridge_keylet = protocol::bridge_keylet_from_door_issue(
            Uint160::from_void(bridge_spec.door(chain_type).data()),
            *bridge_spec.issue(chain_type).get::<Issue>(),
        );
        let sle = view.read(bridge_keylet).map_err(|_| Ter::TEF_BAD_LEDGER)?;
        if let Some(sle) = sle
            && sle.get_field_xchain_bridge(sf("sfXChainBridge")) == *bridge_spec
        {
            return Ok(Some(sle));
        }
        Ok(None)
    };

    if let Some(result) = try_get(XChainBridgeChainType::Locking)? {
        return Ok(Some(result));
    }
    try_get(XChainBridgeChainType::Issuing)
}

pub fn apply_xchain_commit<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let mut psb = ledger::FlowSandbox::new(view);

    let account = sttx.get_account_id(sf("sfAccount"));
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));

    let sle_account = match psb.peek(protocol::account_keylet(Uint160::from_void(account.data()))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let submitting_account_info =
        pre_fee_balance_drops.map(|pre_fee_balance| TransferHelperSubmittingAccountInfo {
            account,
            pre_fee_balance: STAmount::from_xrp_amount(XRPAmount::from_drops(pre_fee_balance)),
            post_fee_balance: sle_account.get_field_amount(sf("sfBalance")),
        });

    let sle_bridge = match read_bridge_helper(&psb, &bridge_spec) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };

    let dst = sle_bridge.get_account_id(sf("sfAccount"));

    let ter = transfer_helper(
        &mut psb,
        &account,
        &dst,
        None,
        None,
        &amount,
        false,
        DepositAuthPolicy::Normal,
        submitting_account_info.as_ref(),
    );

    if !protocol::is_tes_success(ter) {
        return ter;
    }

    psb.apply()
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

pub fn apply_xchain_account_create_commit<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let mut psb = ledger::FlowSandbox::new(view);

    let account = sttx.get_account_id(sf("sfAccount"));
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let reward = sttx.get_field_amount(sf("sfSignatureReward"));
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));

    let sle_account = match psb.peek(protocol::account_keylet(Uint160::from_void(account.data()))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let submitting_account_info =
        pre_fee_balance_drops.map(|pre_fee_balance| TransferHelperSubmittingAccountInfo {
            account,
            pre_fee_balance: STAmount::from_xrp_amount(XRPAmount::from_drops(pre_fee_balance)),
            post_fee_balance: sle_account.get_field_amount(sf("sfBalance")),
        });

    let sle_bridge_arc = match read_bridge_helper(&psb, &bridge_spec) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };

    let dst = sle_bridge_arc.get_account_id(sf("sfAccount"));

    let to_transfer = amount.clone() + reward;

    let ter = transfer_helper(
        &mut psb,
        &account,
        &dst,
        None,
        None,
        &to_transfer,
        true,
        DepositAuthPolicy::Normal,
        submitting_account_info.as_ref(),
    );

    if !protocol::is_tes_success(ter) {
        return ter;
    }

    let mut sle_bridge = (*sle_bridge_arc).clone();
    let count = sle_bridge.get_field_u64(sf("sfXChainAccountCreateCount"));
    sle_bridge.set_field_u64(sf("sfXChainAccountCreateCount"), count + 1);
    if psb.update(Arc::new(sle_bridge)).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }

    psb.apply()
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DepositAuthPolicy {
    Normal,
    DstCanBypass,
}

#[derive(Debug, Clone)]
struct TransferHelperSubmittingAccountInfo {
    account: AccountID,
    pre_fee_balance: STAmount,
    post_fee_balance: STAmount,
}

fn transfer_helper<V: ApplyView>(
    psb: &mut V,
    src: &AccountID,
    dst: &AccountID,
    dst_tag: Option<u32>,
    claim_owner: Option<&AccountID>,
    amt: &STAmount,
    can_create: bool,
    deposit_auth_policy: DepositAuthPolicy,
    submitting_account_info: Option<&TransferHelperSubmittingAccountInfo>,
) -> Ter {
    if src == dst {
        return Ter::TES_SUCCESS;
    }

    let dst_keylet = protocol::account_keylet(Uint160::from_void(dst.data()));
    if let Some(sle_dst) = match psb.peek(dst_keylet) {
        Ok(value) => value,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    } {
        let flags = sle_dst.get_field_u32(sf("sfFlags"));
        if (flags & 0x0002_0000) != 0 && dst_tag.is_none() {
            return Ter::TEC_DST_TAG_NEEDED;
        }

        let can_bypass_deposit_auth =
            claim_owner == Some(dst) && deposit_auth_policy == DepositAuthPolicy::DstCanBypass;
        if !can_bypass_deposit_auth && (flags & 0x0100_0000) != 0 {
            let preauth_keylet = protocol::deposit_preauth_keylet(
                Uint160::from_void(dst.data()),
                Uint160::from_void(src.data()),
            );
            match psb.exists(preauth_keylet) {
                Ok(true) => {}
                Ok(false) => return Ter::TEC_NO_PERMISSION,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            }
        }
    } else if !amt.native() || !can_create {
        return Ter::TEC_NO_DST;
    }

    if amt.native() {
        let src_keylet = protocol::account_keylet(Uint160::from_void(src.data()));
        let sle_src_arc = match psb.peek(src_keylet) {
            Ok(Some(sle)) => sle,
            Ok(None) => return Ter::TEC_INTERNAL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };

        let reserve = ledger::effective_account_reserve(psb.fees(), &sle_src_arc, 0, 0);
        let cur_balance = sle_src_arc.get_field_amount(sf("sfBalance"));
        let available_balance = match submitting_account_info {
            Some(info) if info.account == *src && info.post_fee_balance == cur_balance => {
                info.pre_fee_balance.xrp()
            }
            _ => cur_balance.xrp(),
        };
        let cur_bal = cur_balance.xrp();

        if available_balance.drops() < amt.xrp().drops() + (reserve as i64) {
            return Ter::TEC_UNFUNDED_PAYMENT;
        }

        let sle_dst_arc = match psb.peek(dst_keylet) {
            Ok(Some(sle)) => Some(sle),
            Ok(None) => None,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };

        let mut sle_dst = if let Some(sle) = sle_dst_arc {
            (*sle).clone()
        } else {
            if amt.xrp().drops() < (psb.fees().reserve as i64) {
                return Ter::TEC_NO_DST_INSUF_XRP;
            }
            let mut sle = STLedgerEntry::new(dst_keylet);
            sle.set_account_id(sf("sfAccount"), *dst);
            sle.set_field_u32(sf("sfSequence"), psb.seq());
            if psb.insert(Arc::new(sle.clone())).is_err() {
                return Ter::TEF_BAD_LEDGER;
            }
            sle
        };

        let mut sle_src = (*sle_src_arc).clone();

        let new_src_bal = XRPAmount::from_drops(cur_bal.drops() - amt.xrp().drops());
        let new_dst_bal = XRPAmount::from_drops(
            sle_dst.get_field_amount(sf("sfBalance")).xrp().drops() + amt.xrp().drops(),
        );

        sle_src.set_field_amount(sf("sfBalance"), STAmount::from_xrp_amount(new_src_bal));
        sle_dst.set_field_amount(sf("sfBalance"), STAmount::from_xrp_amount(new_dst_bal));

        if psb.update(Arc::new(sle_src)).is_err() || psb.update(Arc::new(sle_dst)).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }

        return Ter::TES_SUCCESS;
    }

    let paths = protocol::STPathSet::new(sf("sfPaths"));
    let (strand_ter, strands) = ledger::flow_engine::strand_builder::to_strands_checked(
        psb,
        src,
        dst,
        &amt.asset(),
        None,
        &paths,
        true,
        true,
        false,
    );
    if strand_ter != Ter::TES_SUCCESS {
        return if protocol::is_tec_claim(strand_ter) || protocol::is_ter_retry(strand_ter) {
            strand_ter
        } else {
            Ter::TEC_XCHAIN_PAYMENT_FAILED
        };
    }
    let result = ledger::flow_engine::strand_flow::execute_strands(
        psb,
        &strands,
        amt,
        false,
        ledger::ripple_calc::OfferCrossing::No,
        None,
        src,
        dst,
        None,
        None,
    );
    if protocol::is_tes_success(result.ter)
        || protocol::is_tec_claim(result.ter)
        || protocol::is_ter_retry(result.ter)
    {
        result.ter
    } else {
        Ter::TEC_XCHAIN_PAYMENT_FAILED
    }
}

pub fn apply_xchain_claim<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let mut psb = ledger::FlowSandbox::new(view);

    let account = sttx.get_account_id(sf("sfAccount"));
    let dst = sttx.get_account_id(sf("sfDestination"));
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let this_chain_amount = sttx.get_field_amount(sf("sfAmount"));
    let claim_id = sttx.get_field_u64(sf("sfXChainClaimID"));
    let claim_id_keylet = protocol::xchain_owned_claim_id_keylet_from_bridge(
        Uint160::from_void(bridge_spec.locking_chain_door().data()),
        *bridge_spec.locking_chain_issue().get::<Issue>(),
        Uint160::from_void(bridge_spec.issuing_chain_door().data()),
        *bridge_spec.issuing_chain_issue().get::<Issue>(),
        claim_id,
    );

    let sle_bridge = match read_bridge_helper(&psb, &bridge_spec) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };
    let sle_claim_id = match psb.peek(claim_id_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };

    let this_door = sle_bridge.get_account_id(sf("sfAccount"));
    let dst_chain = if this_door == bridge_spec.locking_chain_door() {
        XChainBridgeChainType::Locking
    } else {
        XChainBridgeChainType::Issuing
    };
    let src_chain = STXChainBridge::other_chain(dst_chain);

    let mut sending_amount = this_chain_amount.clone();
    sending_amount.set_issue(bridge_spec.issue(src_chain));

    let (signers_list, quorum, sl_ter) = get_signers_list_and_quorum(&psb, &sle_bridge);
    if !protocol::is_tes_success(sl_ter) {
        return sl_ter;
    }

    let mut cur_atts = match XChainClaimAttestations::from_st_array(
        &sle_claim_id.get_field_array(sf("sfXChainClaimAttestations")),
        XChainClaimAttestation::from_st_object,
    ) {
        Ok(attestations) => attestations,
        Err(_) => return Ter::TEC_INTERNAL,
    };

    let claim_r = on_claim(
        &psb,
        &mut cur_atts,
        &sending_amount,
        src_chain == XChainBridgeChainType::Locking,
        quorum,
        &signers_list,
    );

    let reward_accounts = match claim_r {
        Ok(accs) => accs,
        Err(ter) => return ter,
    };

    let reward_pool_src = sle_claim_id.get_account_id(sf("sfAccount"));
    let signature_reward = sle_claim_id.get_field_amount(sf("sfSignatureReward"));
    let dst_tag = if sttx.is_field_present(sf("sfDestinationTag")) {
        Some(sttx.get_field_u32(sf("sfDestinationTag")))
    } else {
        None
    };

    let r = finalize_claim_helper(
        &mut psb,
        &bridge_spec,
        &dst,
        dst_tag,
        &account,
        &sending_amount,
        &reward_pool_src,
        &signature_reward,
        &reward_accounts,
        src_chain,
        &claim_id_keylet,
        OnTransferFail::KeepClaim,
        DepositAuthPolicy::DstCanBypass,
    );

    if !r.is_tes_success() {
        return r.ter();
    }

    psb.apply()
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

fn get_signers_list_and_quorum<V: ReadView>(
    view: &V,
    sle_bridge: &STLedgerEntry,
) -> (HashMap<AccountID, u32>, u32, Ter) {
    let mut r = HashMap::new();
    let this_door = sle_bridge.get_account_id(sf("sfAccount"));
    let door_keylet = protocol::account_keylet(Uint160::from_void(this_door.data()));
    match view.read(door_keylet) {
        Ok(Some(_)) => {}
        Ok(None) => return (r, u32::MAX, Ter::TEC_INTERNAL),
        Err(_) => return (r, u32::MAX, Ter::TEF_BAD_LEDGER),
    }

    let signers_keylet = protocol::keylet::signers(Uint160::from_void(this_door.data()));
    let sle_s = match view.read(signers_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return (r, u32::MAX, Ter::TEC_XCHAIN_NO_SIGNERS_LIST),
        Err(_) => return (r, u32::MAX, Ter::TEF_BAD_LEDGER),
    };

    let quorum = sle_s.get_field_u32(sf("sfSignerQuorum"));
    let signer_entries = sle_s.get_field_array(sf("sfSignerEntries"));

    for entry in signer_entries.iter() {
        let account = entry.get_account_id(sf("sfAccount"));
        let weight = entry.get_field_u16(sf("sfSignerWeight")) as u32;
        r.insert(account, weight);
    }

    (r, quorum, Ter::TES_SUCCESS)
}

fn check_attestation_public_key<V: ApplyView>(
    view: &V,
    signers_list: &HashMap<AccountID, u32>,
    attestation_signer_account: AccountID,
    public_key: &PublicKey,
) -> Ter {
    if !signers_list.contains_key(&attestation_signer_account) {
        return Ter::TEC_NO_PERMISSION;
    }

    let account_from_pk = calc_account_id(public_key.as_bytes());
    let account_keylet =
        protocol::account_keylet(Uint160::from_void(attestation_signer_account.data()));
    let account_sle = match view.read(account_keylet) {
        Ok(account_sle) => account_sle,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };

    if let Some(account_sle) = account_sle {
        if account_from_pk == attestation_signer_account {
            if account_sle.get_field_u32(sf("sfFlags")) & lsfDisableMaster != 0 {
                return Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR;
            }
        } else {
            let regular_key_field = sf("sfRegularKey");
            if !account_sle.is_field_present(regular_key_field)
                || account_sle.get_account_id(regular_key_field) != account_from_pk
            {
                return Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR;
            }
        }
    } else if account_from_pk != attestation_signer_account {
        return Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnTransferFail {
    KeepClaim,
    RemoveClaim,
}

struct FinalizeClaimHelperResult {
    main_funds_ter: Option<Ter>,
    reward_ter: Option<Ter>,
    rm_sle_ter: Option<Ter>,
}

impl FinalizeClaimHelperResult {
    fn is_tes_success(&self) -> bool {
        self.main_funds_ter.is_none_or(protocol::is_tes_success)
            && self.reward_ter.is_none_or(protocol::is_tes_success)
            && self.rm_sle_ter.is_none_or(protocol::is_tes_success)
    }

    fn ter(&self) -> Ter {
        if self.is_tes_success() {
            return Ter::TES_SUCCESS;
        }
        if let Some(t) = self.main_funds_ter
            && (protocol::is_tef_failure(t) || t == Ter::TEC_INTERNAL)
        {
            return t;
        }
        if let Some(t) = self.reward_ter
            && (protocol::is_tef_failure(t) || t == Ter::TEC_INTERNAL)
        {
            return t;
        }
        if let Some(t) = self.rm_sle_ter
            && (protocol::is_tef_failure(t) || t == Ter::TEC_INTERNAL)
        {
            return t;
        }
        if let Some(t) = self.main_funds_ter {
            if !protocol::is_tes_success(t) {
                return t;
            }
        }
        if let Some(t) = self.reward_ter {
            if !protocol::is_tes_success(t) {
                return t;
            }
        }
        if let Some(t) = self.rm_sle_ter {
            if !protocol::is_tes_success(t) {
                return t;
            }
        }
        Ter::TES_SUCCESS
    }
}

fn finalize_claim_helper<V: ApplyView>(
    outer_sb: &mut V,
    bridge_spec: &STXChainBridge,
    dst: &AccountID,
    dst_tag: Option<u32>,
    claim_owner: &AccountID,
    sending_amount: &STAmount,
    reward_pool_src: &AccountID,
    reward_pool: &STAmount,
    reward_accounts: &[AccountID],
    src_chain: XChainBridgeChainType,
    claim_id_keylet: &Keylet,
    on_transfer_fail: OnTransferFail,
    deposit_auth_policy: DepositAuthPolicy,
) -> FinalizeClaimHelperResult {
    let mut result = FinalizeClaimHelperResult {
        main_funds_ter: None,
        reward_ter: None,
        rm_sle_ter: None,
    };

    let dst_chain = STXChainBridge::other_chain(src_chain);
    let mut this_chain_amount = sending_amount.clone();
    this_chain_amount.set_issue(bridge_spec.issue(dst_chain));
    let this_door = bridge_spec.door(dst_chain);

    {
        let mut inner_sb = ledger::FlowSandbox::new(outer_sb);

        let main_funds_ter = transfer_helper(
            &mut inner_sb,
            &this_door,
            dst,
            dst_tag,
            Some(claim_owner),
            &this_chain_amount,
            true,
            deposit_auth_policy,
            None,
        );
        result.main_funds_ter = Some(main_funds_ter);

        if !protocol::is_tes_success(main_funds_ter)
            && on_transfer_fail == OnTransferFail::KeepClaim
        {
            return result;
        }

        let reward_ter = if reward_accounts.is_empty() {
            Ter::TES_SUCCESS
        } else {
            let num_rewards = reward_accounts.len() as u64;
            let den = STAmount::new_with_asset(
                sf("sfGeneric"),
                reward_pool.asset(),
                num_rewards,
                0,
                false,
            );
            let _rounding = inner_sb
                .rules()
                .enabled(&protocol::feature_id("fixXChainRewardRounding"))
                .then(|| NumberRoundModeGuard::new(RoundingMode::Downward));
            let share = reward_pool.divide(&den, reward_pool.asset());

            for ra in reward_accounts {
                let th_ter = transfer_helper(
                    &mut inner_sb,
                    reward_pool_src,
                    ra,
                    None,
                    None,
                    &share,
                    false,
                    DepositAuthPolicy::Normal,
                    None,
                );
                if th_ter == Ter::TEC_UNFUNDED_PAYMENT || th_ter == Ter::TEC_INTERNAL {
                    return FinalizeClaimHelperResult {
                        main_funds_ter: result.main_funds_ter,
                        reward_ter: Some(th_ter),
                        rm_sle_ter: None,
                    };
                }
            }
            Ter::TES_SUCCESS
        };
        result.reward_ter = Some(reward_ter);

        if !protocol::is_tes_success(reward_ter)
            && (on_transfer_fail == OnTransferFail::KeepClaim || reward_ter == Ter::TEC_INTERNAL)
        {
            return result;
        }

        if !protocol::is_tes_success(main_funds_ter) || protocol::is_tes_success(reward_ter) {
            if inner_sb.apply().is_err() {
                result.rm_sle_ter = Some(Ter::TEF_BAD_LEDGER);
                return result;
            }
        }
    }

    let sle_claim_id = match outer_sb.peek(*claim_id_keylet) {
        Ok(value) => value,
        Err(_) => {
            result.rm_sle_ter = Some(Ter::TEF_BAD_LEDGER);
            return result;
        }
    };
    if let Some(sle_claim_id) = sle_claim_id {
        let cid_owner = sle_claim_id.get_account_id(sf("sfAccount"));
        let sle_owner = match outer_sb.peek(protocol::account_keylet(Uint160::from_void(
            cid_owner.data(),
        ))) {
            Ok(Some(value)) => value,
            Ok(None) | Err(_) => {
                result.rm_sle_ter = Some(Ter::TEF_BAD_LEDGER);
                return result;
            }
        };
        let page = sle_claim_id.get_field_u64(sf("sfOwnerNode"));

        match dir_remove(
            outer_sb as &mut dyn ApplyView,
            &protocol::owner_dir_keylet(Uint160::from_void(cid_owner.data())),
            page,
            claim_id_keylet.key,
            true,
        ) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                result.rm_sle_ter = Some(Ter::TEF_BAD_LEDGER);
                return result;
            }
        }
        if ledger::decrease_owner_count_for_object(outer_sb, &sle_owner, &sle_claim_id, 1).is_err()
            || outer_sb.erase(sle_claim_id).is_err()
        {
            result.rm_sle_ter = Some(Ter::TEF_BAD_LEDGER);
            return result;
        }
    }

    result
}

fn on_claim<V: ApplyView>(
    view: &V,
    attestations: &mut XChainClaimAttestations,
    sending_amount: &STAmount,
    was_locking_chain_send: bool,
    quorum: u32,
    signers_list: &HashMap<AccountID, u32>,
) -> Result<Vec<AccountID>, Ter> {
    attestations.erase_if(|att| {
        check_attestation_public_key(view, signers_list, att.key_account, &att.public_key)
            != Ter::TES_SUCCESS
    });
    let mut reward_accounts = Vec::new();
    let mut weight = 0;
    for att in attestations.attestations() {
        if att.match_fields(sending_amount, was_locking_chain_send, None)
            != protocol::AttestationMatch::NonDstMismatch
        {
            if let Some(w) = signers_list.get(&att.key_account) {
                weight += *w;
                reward_accounts.push(att.reward_account);
            }
        }
    }

    if weight >= quorum {
        Ok(reward_accounts)
    } else {
        Err(Ter::TEC_XCHAIN_CLAIM_NO_QUORUM)
    }
}

pub fn apply_xchain_add_claim_attestation<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let mut psb = ledger::FlowSandbox::new(view);
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let sle_bridge = match read_bridge_helper(&psb, &bridge_spec) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_NO_ENTRY,
        Err(ter) => return ter,
    };

    let (signers_list, quorum, sl_ter) = get_signers_list_and_quorum(&psb, &sle_bridge);
    if !protocol::is_tes_success(sl_ter) {
        return sl_ter;
    }

    let att = attestations::AttestationClaim::from_transaction_st_object(sttx);
    let claim_id = att.claim_id;
    let claim_id_keylet = protocol::xchain_owned_claim_id_keylet_from_bridge(
        Uint160::from_void(bridge_spec.locking_chain_door().data()),
        *bridge_spec.locking_chain_issue().get::<Issue>(),
        Uint160::from_void(bridge_spec.issuing_chain_door().data()),
        *bridge_spec.issuing_chain_issue().get::<Issue>(),
        claim_id,
    );
    let sle_claim_id_arc = match psb.peek(claim_id_keylet) {
        Ok(Some(sle)) => sle,
        _ => return Ter::TEC_XCHAIN_NO_CLAIM_ID,
    };

    let signer = att.base.attestation_signer_account;
    let signer_key_ter =
        check_attestation_public_key(&psb, &signers_list, signer, &att.base.public_key);
    if !protocol::is_tes_success(signer_key_ter) {
        return signer_key_ter;
    }

    if sle_claim_id_arc.get_account_id(sf("sfOtherChainSource")) != att.base.sending_account {
        return Ter::TEC_XCHAIN_SENDING_ACCOUNT_MISMATCH;
    }

    let this_door = sle_bridge.get_account_id(sf("sfAccount"));
    let dst_chain = if this_door == bridge_spec.locking_chain_door() {
        XChainBridgeChainType::Locking
    } else if this_door == bridge_spec.issuing_chain_door() {
        XChainBridgeChainType::Issuing
    } else {
        return Ter::TEC_INTERNAL;
    };
    let src_chain = STXChainBridge::other_chain(dst_chain);
    if STXChainBridge::dst_chain(att.base.was_locking_chain_send) != dst_chain {
        return Ter::TEC_XCHAIN_WRONG_CHAIN;
    }

    let mut cur_atts = match XChainClaimAttestations::from_st_array(
        &sle_claim_id_arc.get_field_array(sf("sfXChainClaimAttestations")),
        XChainClaimAttestation::from_st_object,
    ) {
        Ok(attestations) => attestations,
        Err(_) => return Ter::TEC_INTERNAL,
    };
    cur_atts.erase_if(|existing| {
        check_attestation_public_key(
            &psb,
            &signers_list,
            existing.key_account,
            &existing.public_key,
        ) != Ter::TES_SUCCESS
    });
    let new_attestation = XChainClaimAttestation::from_signed(&att);
    let mut replaced = false;
    for existing in cur_atts.attestations_mut() {
        if existing.key_account == new_attestation.key_account {
            *existing = new_attestation.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        cur_atts.emplace_back(new_attestation);
    }

    let mut reward_accounts = Vec::new();
    let mut weight = 0_u32;
    for existing in cur_atts.attestations() {
        if existing.match_fields(
            &att.base.sending_amount,
            att.base.was_locking_chain_send,
            att.dst,
        ) == protocol::AttestationMatch::Match
            && let Some(signer_weight) = signers_list.get(&existing.key_account)
        {
            weight += *signer_weight;
            reward_accounts.push(existing.reward_account);
        }
    }

    let claim_owner = sle_claim_id_arc.get_account_id(sf("sfAccount"));
    let reward_amount = sle_claim_id_arc.get_field_amount(sf("sfSignatureReward"));
    let mut updated = (*sle_claim_id_arc).clone();
    updated.set_field_array(sf("sfXChainClaimAttestations"), cur_atts.to_st_array());
    if psb.update(Arc::new(updated)).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }

    if weight >= quorum
        && let Some(dst) = att.dst
    {
        let result = finalize_claim_helper(
            &mut psb,
            &bridge_spec,
            &dst,
            None,
            &claim_owner,
            &att.base.sending_amount,
            &claim_owner,
            &reward_amount,
            &reward_accounts,
            src_chain,
            &claim_id_keylet,
            OnTransferFail::KeepClaim,
            DepositAuthPolicy::Normal,
        );
        let ter = result.ter();
        if ter == Ter::TEC_INTERNAL || protocol::is_tef_failure(ter) {
            return ter;
        }
    }

    psb.apply()
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

pub fn apply_xchain_add_account_create_attestation<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let sle_bridge = match read_bridge_helper(view, &bridge_spec) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_NO_ENTRY,
        Err(ter) => return ter,
    };

    let (signers_list, quorum, sl_ter) = get_signers_list_and_quorum(view, &sle_bridge);
    if !protocol::is_tes_success(sl_ter) {
        return sl_ter;
    }

    let signer = sttx.get_account_id(sf("sfAttestationSignerAccount"));
    let public_key = match PublicKey::from_slice(&sttx.get_field_vl(sf("sfPublicKey"))) {
        Ok(public_key) => public_key,
        Err(_) => return Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR,
    };
    let signer_key_ter = check_attestation_public_key(view, &signers_list, signer, &public_key);
    if !protocol::is_tes_success(signer_key_ter) {
        return signer_key_ter;
    }

    let this_door = sle_bridge.get_account_id(sf("sfAccount"));
    let dst_chain = if this_door == bridge_spec.locking_chain_door() {
        XChainBridgeChainType::Locking
    } else if this_door == bridge_spec.issuing_chain_door() {
        XChainBridgeChainType::Issuing
    } else {
        return Ter::TEC_INTERNAL;
    };
    let src_chain = STXChainBridge::other_chain(dst_chain);

    let att = attestations::AttestationCreateAccount::from_transaction_st_object(sttx);
    if STXChainBridge::dst_chain(att.base.was_locking_chain_send) != dst_chain {
        return Ter::TEC_XCHAIN_WRONG_CHAIN;
    }

    let mut psb = ledger::FlowSandbox::new(view);
    let mut sle_bridge_mut = match read_bridge_helper(&psb, &bridge_spec) {
        Ok(Some(sle)) => (*sle).clone(),
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };

    let claim_count = sle_bridge_mut.get_field_u64(sf("sfXChainAccountClaimCount"));
    if att.create_count <= claim_count {
        return Ter::TEC_XCHAIN_ACCOUNT_CREATE_PAST;
    }
    if att.create_count
        >= claim_count + tx::utility::x_chain_bridge::XBRIDGE_MAX_ACCOUNT_CREATE_CLAIMS as u64
    {
        return Ter::TEC_XCHAIN_ACCOUNT_CREATE_TOO_MANY;
    }

    let claim_id_keylet = protocol::xchain_owned_create_account_claim_id_keylet_from_bridge(
        Uint160::from_void(bridge_spec.locking_chain_door().data()),
        *bridge_spec.locking_chain_issue().get::<Issue>(),
        Uint160::from_void(bridge_spec.issuing_chain_door().data()),
        *bridge_spec.issuing_chain_issue().get::<Issue>(),
        att.create_count,
    );

    let sle_claim_id_arc = match psb.peek(claim_id_keylet) {
        Ok(sle) => sle,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let create_claim_id = sle_claim_id_arc.is_none();

    if create_claim_id {
        let sle_door = match psb.peek(protocol::account_keylet(Uint160::from_void(
            this_door.data(),
        ))) {
            Ok(Some(sle)) => sle,
            Ok(None) => return Ter::TEC_INTERNAL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        let reserve = ledger::effective_account_reserve(psb.fees(), &sle_door, 1, 0);
        if sle_door.get_field_amount(sf("sfBalance")).xrp().drops() < reserve as i64 {
            return Ter::TEC_INSUFFICIENT_RESERVE;
        }
    }

    let mut attestations = match sle_claim_id_arc.as_ref() {
        Some(sle_claim_id) => match XChainCreateAccountAttestations::from_st_array(
            &sle_claim_id.get_field_array(sf("sfXChainCreateAccountAttestations")),
            XChainCreateAccountAttestation::from_st_object,
        ) {
            Ok(atts) => atts,
            Err(_) => return Ter::TEC_INTERNAL,
        },
        None => XChainCreateAccountAttestations::new(Vec::new()),
    };
    attestations.erase_if(|existing| {
        check_attestation_public_key(
            &psb,
            &signers_list,
            existing.key_account,
            &existing.public_key,
        ) != Ter::TES_SUCCESS
    });
    let new_attestation = XChainCreateAccountAttestation::from_signed(&att);
    let mut replaced = false;
    for existing in attestations.attestations_mut().iter_mut() {
        if existing.key_account == new_attestation.key_account {
            *existing = new_attestation.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        attestations.emplace_back(new_attestation.clone());
    }

    let mut reward_accounts = Vec::new();
    let mut weight = 0_u32;
    for existing in attestations.attestations() {
        if existing.match_fields(
            &att.base.sending_amount,
            &att.reward_amount,
            att.base.was_locking_chain_send,
            att.to_create,
        ) == protocol::AttestationMatch::Match
            && let Some(signer_weight) = signers_list.get(&existing.key_account)
        {
            weight += *signer_weight;
            reward_accounts.push(existing.reward_account);
        }
    }
    let has_quorum = weight >= quorum;

    if let Some(sle_claim_id) = sle_claim_id_arc {
        let mut updated = (*sle_claim_id).clone();
        updated.set_field_array(
            sf("sfXChainCreateAccountAttestations"),
            attestations.to_st_array(),
        );
        if psb.update(Arc::new(updated)).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }
    }

    if has_quorum && claim_count + 1 == att.create_count {
        let result = finalize_claim_helper(
            &mut psb,
            &bridge_spec,
            &att.to_create,
            None,
            &this_door,
            &att.base.sending_amount,
            &this_door,
            &att.reward_amount,
            &reward_accounts,
            src_chain,
            &claim_id_keylet,
            OnTransferFail::RemoveClaim,
            DepositAuthPolicy::Normal,
        );
        let ter = result.ter();
        if ter == Ter::TEC_INTERNAL
            || ter == Ter::TEC_UNFUNDED_PAYMENT
            || protocol::is_tef_failure(ter)
        {
            return ter;
        }

        sle_bridge_mut.set_field_u64(sf("sfXChainAccountClaimCount"), att.create_count);
        if psb.update(Arc::new(sle_bridge_mut)).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }
    } else if create_claim_id {
        let mut sle_claim_id = STLedgerEntry::new(claim_id_keylet);
        sle_claim_id.set_account_id(sf("sfAccount"), this_door);
        sle_claim_id.set_field_xchain_bridge(sf("sfXChainBridge"), bridge_spec);
        sle_claim_id.set_field_u64(sf("sfXChainAccountCreateCount"), att.create_count);
        sle_claim_id.set_field_array(
            sf("sfXChainCreateAccountAttestations"),
            attestations.to_st_array(),
        );

        let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(this_door.data()));
        let describe = |obj: &mut STObject| {
            obj.set_account_id(sf("sfOwner"), this_door);
        };
        let page = match dir_insert(
            &mut psb as &mut dyn ApplyView,
            &owner_dir,
            claim_id_keylet.key,
            &describe,
        ) {
            Ok(Some(page)) => page,
            Ok(None) => return Ter::TEC_DIR_FULL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        sle_claim_id.set_field_u64(sf("sfOwnerNode"), page);

        let sle_door = match psb.peek(protocol::account_keylet(Uint160::from_void(
            this_door.data(),
        ))) {
            Ok(Some(sle)) => sle,
            Ok(None) => return Ter::TEC_INTERNAL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        if adjust_owner_count(&mut psb, &sle_door, 1).is_err()
            || psb.insert(Arc::new(sle_claim_id)).is_err()
        {
            return Ter::TEF_BAD_LEDGER;
        }
    }

    psb.apply()
        .map_or(Ter::TEF_BAD_LEDGER, |_| Ter::TES_SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::base_uint::Uint256;
    use ledger::{ApplyViewImpl, Ledger, RawView, ReadViewTx, ViewError};
    use protocol::{ApplyFlags, Currency, Issue, LedgerEntryType, Rules, TxType};

    #[derive(Debug)]
    struct FaultReadView {
        base: Arc<Ledger>,
    }

    impl ReadView for FaultReadView {
        fn open(&self) -> bool {
            ReadView::open(self.base.as_ref())
        }
        fn header(&self) -> ledger::LedgerHeader {
            ReadView::header(self.base.as_ref())
        }
        fn fees(&self) -> ledger::Fees {
            ReadView::fees(self.base.as_ref())
        }
        fn rules(&self) -> Rules {
            ReadView::rules(self.base.as_ref())
        }
        fn exists(&self, _key: Keylet) -> Result<bool, ViewError> {
            Err(ViewError::Conversion("injected exists failure".into()))
        }
        fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            ReadView::succ(self.base.as_ref(), key, last)
        }
        fn read(&self, _key: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Err(ViewError::Conversion("injected read failure".into()))
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            ReadView::sles(self.base.as_ref())
        }
        fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
            ReadView::tx_exists(self.base.as_ref(), key)
        }
        fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            ReadView::tx_read(self.base.as_ref(), key)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            ReadView::txs(self.base.as_ref())
        }
    }

    fn bridge() -> STXChainBridge {
        STXChainBridge::from_parts(
            AccountID::from_array([0x11; 20]),
            Issue::new(Currency::from_u64(1), AccountID::from_array([0x12; 20])),
            AccountID::from_array([0x13; 20]),
            Issue::new(Currency::from_u64(2), AccountID::from_array([0x14; 20])),
        )
    }

    fn account_root(account: AccountID) -> STLedgerEntry {
        let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
        let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
        sle.set_account_id(sf("sfAccount"), account);
        sle.set_field_amount(sf("sfBalance"), STAmount::new_native(10_000_000_000, false));
        sle.set_field_u32(sf("sfSequence"), 1);
        sle.set_field_u32(sf("sfOwnerCount"), 0);
        sle.set_field_u32(sf("sfFlags"), 0);
        sle
    }

    #[test]
    fn xchain_created_objects_preserve_the_incremented_owner_count() {
        let bridge = bridge();
        let account = bridge.locking_chain_door();
        let mut ledger = Ledger::from_ledger_seq_and_close_time(1, 0, false);
        ledger
            .raw_insert(Arc::new(account_root(account)))
            .expect("seed bridge owner");
        let mut apply = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

        let create_bridge = STTx::new(TxType::XCHAIN_CREATE_BRIDGE, |object| {
            object.set_account_id(sf("sfAccount"), account);
            object.set_field_xchain_bridge(sf("sfXChainBridge"), bridge.clone());
            object.set_field_amount(sf("sfSignatureReward"), STAmount::new_native(10, false));
        });
        assert_eq!(
            apply_xchain_create_bridge(&mut apply, &create_bridge),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            apply
                .read(protocol::account_keylet(Uint160::from_void(account.data())))
                .expect("read account")
                .expect("account exists")
                .get_field_u32(sf("sfOwnerCount")),
            1
        );

        let create_claim = STTx::new(TxType::XCHAIN_CREATE_CLAIM_ID, |object| {
            object.set_account_id(sf("sfAccount"), account);
            object.set_field_xchain_bridge(sf("sfXChainBridge"), bridge.clone());
            object.set_field_amount(sf("sfSignatureReward"), STAmount::new_native(10, false));
            object.set_account_id(sf("sfOtherChainSource"), AccountID::from_array([0x21; 20]));
        });
        assert_eq!(
            apply_xchain_create_claim_id(&mut apply, &create_claim),
            Ter::TES_SUCCESS
        );
        assert_eq!(
            apply
                .read(protocol::account_keylet(Uint160::from_void(account.data())))
                .expect("read account")
                .expect("account exists")
                .get_field_u32(sf("sfOwnerCount")),
            2
        );
    }

    #[test]
    fn xchain_storage_reads_fail_hard_instead_of_becoming_missing_objects() {
        let bridge = bridge();
        let faulty = Arc::new(FaultReadView {
            base: Arc::new(Ledger::from_ledger_seq_and_close_time(1, 0, false)),
        });
        assert!(matches!(
            read_bridge_helper(faulty.as_ref(), &bridge),
            Err(Ter::TEF_BAD_LEDGER)
        ));

        let mut bridge_entry = STLedgerEntry::from_type_and_key(
            LedgerEntryType::Bridge,
            protocol::bridge_keylet_from_door_issue(
                Uint160::from_void(bridge.locking_chain_door().data()),
                *bridge.locking_chain_issue().get::<Issue>(),
            )
            .key,
        );
        bridge_entry.set_account_id(sf("sfAccount"), bridge.locking_chain_door());
        assert_eq!(
            get_signers_list_and_quorum(faulty.as_ref(), &bridge_entry).2,
            Ter::TEF_BAD_LEDGER,
            "a door AccountRoot storage failure must not become pinned's missing-door tecINTERNAL"
        );

        let account = bridge.locking_chain_door();
        let tx = STTx::new(TxType::XCHAIN_CREATE_BRIDGE, |object| {
            object.set_account_id(sf("sfAccount"), account);
            object.set_field_xchain_bridge(sf("sfXChainBridge"), bridge.clone());
            object.set_field_amount(sf("sfSignatureReward"), STAmount::new_native(10, false));
        });
        let mut apply = ApplyViewImpl::new(faulty, ApplyFlags::NONE);
        assert_eq!(
            apply_xchain_create_bridge(&mut apply, &tx),
            Ter::TEF_BAD_LEDGER
        );
    }
}
