use basics::math::base_uint::Uint160;
use ledger::{ApplyView, ReadView, adjust_owner_count, dir_insert, dir_remove};
use protocol::{
    AccountID, Issue, Keylet, PublicKey, STAmount, STArray, STLedgerEntry, STObject, STTx,
    STXChainBridge, Ter, XChainBridgeChainType, XChainCreateAccountAttestation,
    XChainCreateAccountAttestations, XRPAmount, attestations, calc_account_id,
    get_field_by_symbol as sf, lsfDisableMaster,
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
        _ => return Ter::TEC_INTERNAL,
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
        _ => return Ter::TEC_DIR_FULL,
    };
    sle_bridge.set_field_u64(sf("sfOwnerNode"), page);

    let _ = adjust_owner_count(view, &sle_acct, 1);
    let _ = view.insert(Arc::new(sle_bridge));
    let _ = view.update(sle_acct);

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
        _ => return Ter::TEC_INTERNAL,
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

    let _ = view.update(Arc::new(sle_bridge));

    Ter::TES_SUCCESS
}

pub fn apply_xchain_create_claim_id<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let reward = sttx.get_field_amount(sf("sfSignatureReward"));
    let other_chain_src = sttx.get_account_id(sf("sfOtherChainSource"));

    let sle_acct = match view.peek(protocol::account_keylet(Uint160::from_void(account.data()))) {
        Ok(Some(sle)) => sle,
        _ => return Ter::TEC_INTERNAL,
    };

    let sle_bridge = match peek_bridge_helper(view, &bridge_spec) {
        Some(sle) => sle,
        None => return Ter::TEC_INTERNAL,
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

    if view.exists(claim_id_keylet).unwrap_or(false) {
        return Ter::TEC_INTERNAL;
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
        _ => return Ter::TEC_DIR_FULL,
    };
    sle_claim_id.set_field_u64(sf("sfOwnerNode"), page);

    let _ = adjust_owner_count(view, &sle_acct, 1);
    let _ = view.insert(Arc::new(sle_claim_id));
    let _ = view.update(Arc::new(updated_bridge));
    let _ = view.update(sle_acct);

    Ter::TES_SUCCESS
}

fn peek_bridge_helper<V: ApplyView>(
    view: &V,
    bridge_spec: &STXChainBridge,
) -> Option<Arc<STLedgerEntry>> {
    let try_get = |chain_type: XChainBridgeChainType| -> Option<Arc<STLedgerEntry>> {
        let bridge_keylet = protocol::bridge_keylet_from_door_issue(
            Uint160::from_void(bridge_spec.door(chain_type).data()),
            *bridge_spec.issue(chain_type).get::<Issue>(),
        );
        if let Ok(Some(sle)) = view.read(bridge_keylet) {
            if sle.get_field_xchain_bridge(sf("sfXChainBridge")) == *bridge_spec {
                return Some(sle);
            }
        }
        None
    };

    if let Some(r) = try_get(XChainBridgeChainType::Locking) {
        return Some(r);
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
        _ => return Ter::TEC_INTERNAL,
    };
    let submitting_account_info =
        pre_fee_balance_drops.map(|pre_fee_balance| TransferHelperSubmittingAccountInfo {
            account,
            pre_fee_balance: STAmount::from_xrp_amount(XRPAmount::from_drops(pre_fee_balance)),
            post_fee_balance: sle_account.get_field_amount(sf("sfBalance")),
        });

    let sle_bridge = match peek_bridge_helper(&psb, &bridge_spec) {
        Some(sle) => sle,
        None => return Ter::TEC_INTERNAL,
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

    let _ = psb.apply();
    Ter::TES_SUCCESS
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
        _ => return Ter::TEC_INTERNAL,
    };
    let submitting_account_info =
        pre_fee_balance_drops.map(|pre_fee_balance| TransferHelperSubmittingAccountInfo {
            account,
            pre_fee_balance: STAmount::from_xrp_amount(XRPAmount::from_drops(pre_fee_balance)),
            post_fee_balance: sle_account.get_field_amount(sf("sfBalance")),
        });

    let sle_bridge_arc = match peek_bridge_helper(&psb, &bridge_spec) {
        Some(sle) => sle,
        None => return Ter::TEC_INTERNAL,
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
    let _ = psb.update(Arc::new(sle_bridge));

    let _ = psb.apply();
    Ter::TES_SUCCESS
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
    if let Ok(Some(sle_dst)) = psb.peek(dst_keylet) {
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
            if !psb.exists(preauth_keylet).unwrap_or(false) {
                return Ter::TEC_NO_PERMISSION;
            }
        }
    } else if !amt.native() || !can_create {
        return Ter::TEC_NO_DST;
    }

    if amt.native() {
        let src_keylet = protocol::account_keylet(Uint160::from_void(src.data()));
        let sle_src_arc = match psb.peek(src_keylet) {
            Ok(Some(sle)) => sle,
            _ => return Ter::TEC_INTERNAL,
        };

        let owner_count = sle_src_arc.get_field_u32(sf("sfOwnerCount"));
        let reserve = psb.fees().account_reserve(owner_count as usize);
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
            Err(_) => return Ter::TEC_INTERNAL,
        };

        let mut sle_dst = if let Some(sle) = sle_dst_arc {
            (*sle).clone()
        } else {
            if amt.xrp().drops() < (psb.fees().reserve as i64) {
                return Ter::TEC_NO_DST_INSUF_XRP;
            }
            let mut sle = STLedgerEntry::new(dst_keylet);
            sle.set_account_id(sf("sfAccount"), *dst);
            sle.set_field_u32(sf("sfSequence"), 1);
            let _ = psb.insert(Arc::new(sle.clone()));
            sle
        };

        let mut sle_src = (*sle_src_arc).clone();

        let new_src_bal = XRPAmount::from_drops(cur_bal.drops() - amt.xrp().drops());
        let new_dst_bal = XRPAmount::from_drops(
            sle_dst.get_field_amount(sf("sfBalance")).xrp().drops() + amt.xrp().drops(),
        );

        sle_src.set_field_amount(sf("sfBalance"), STAmount::from_xrp_amount(new_src_bal));
        sle_dst.set_field_amount(sf("sfBalance"), STAmount::from_xrp_amount(new_dst_bal));

        let _ = psb.update(Arc::new(sle_src));
        let _ = psb.update(Arc::new(sle_dst));

        return Ter::TES_SUCCESS;
    }

    let rc_input = ledger::ripple_calc::RippleCalcInput {
        partial_payment_allowed: false,
        default_paths_allowed: true,
        limit_quality: false,
        is_ledger_open: false,
    };

    match ledger::ripple_calc::ripple_calculate(
        psb,
        amt,
        amt,
        dst,
        src,
        &protocol::STPathSet::default(),
        &rc_input,
    ) {
        Ok(out) => {
            if protocol::is_tes_success(out.result)
                || protocol::is_tec_claim(out.result)
                || protocol::is_ter_retry(out.result)
            {
                return out.result;
            }
            Ter::TEC_XCHAIN_PAYMENT_FAILED
        }
        Err(_) => Ter::TEC_INTERNAL,
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

    let sle_bridge = match peek_bridge_helper(&psb, &bridge_spec) {
        Some(sle) => sle,
        None => return Ter::TEC_INTERNAL,
    };
    let sle_claim_id = match psb.peek(claim_id_keylet) {
        Ok(Some(sle)) => sle,
        _ => return Ter::TEC_INTERNAL,
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

    let mut cur_atts = sle_claim_id
        .get_field_array(sf("sfXChainClaimAttestations"))
        .clone();

    let claim_r = on_claim(
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

    let _ = psb.apply();
    Ter::TES_SUCCESS
}

fn get_signers_list_and_quorum<V: ApplyView>(
    view: &V,
    sle_bridge: &STLedgerEntry,
) -> (HashMap<AccountID, u32>, u32, Ter) {
    let mut r = HashMap::new();
    let this_door = sle_bridge.get_account_id(sf("sfAccount"));

    let signers_keylet = protocol::keylet::signers(Uint160::from_void(this_door.data()));
    let sle_s = match view.read(signers_keylet) {
        Ok(Some(sle)) => sle,
        _ => return (r, 0, Ter::TEC_XCHAIN_NO_SIGNERS_LIST),
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
        Err(_) => return Ter::TEC_INTERNAL,
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

        result.main_funds_ter = Some(transfer_helper(
            &mut inner_sb,
            &this_door,
            dst,
            dst_tag,
            Some(claim_owner),
            &this_chain_amount,
            true,
            deposit_auth_policy,
            None,
        ));

        if !protocol::is_tes_success(result.main_funds_ter.unwrap())
            && on_transfer_fail == OnTransferFail::KeepClaim
        {
            return result;
        }

        result.reward_ter = Some(if reward_accounts.is_empty() {
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
        });

        if !protocol::is_tes_success(result.reward_ter.unwrap())
            && (on_transfer_fail == OnTransferFail::KeepClaim
                || result.reward_ter.unwrap() == Ter::TEC_INTERNAL)
        {
            return result;
        }

        if !protocol::is_tes_success(result.main_funds_ter.unwrap())
            || protocol::is_tes_success(result.reward_ter.unwrap())
        {
            let _ = inner_sb.apply();
        }
    }

    if let Ok(Some(sle_claim_id)) = outer_sb.peek(*claim_id_keylet) {
        let cid_owner = sle_claim_id.get_account_id(sf("sfAccount"));
        let sle_owner = outer_sb
            .peek(protocol::account_keylet(Uint160::from_void(
                cid_owner.data(),
            )))
            .ok()
            .flatten();
        let page = sle_claim_id.get_field_u64(sf("sfOwnerNode"));

        if dir_remove(
            outer_sb as &mut dyn ApplyView,
            &protocol::owner_dir_keylet(Uint160::from_void(cid_owner.data())),
            page,
            claim_id_keylet.key,
            true,
        )
        .is_ok()
        {
            let _ = outer_sb.erase(sle_claim_id);
            if let Some(so) = sle_owner {
                let _ = adjust_owner_count(outer_sb, &so, -1);
            }
        } else {
            result.rm_sle_ter = Some(Ter::TEF_BAD_LEDGER);
            return result;
        }
    }

    result
}

fn on_claim(
    attestations: &mut STArray,
    sending_amount: &STAmount,
    was_locking_chain_send: bool,
    quorum: u32,
    signers_list: &HashMap<AccountID, u32>,
) -> Result<Vec<AccountID>, Ter> {
    let mut reward_accounts = Vec::new();
    let mut weight = 0;
    for att in attestations.iter() {
        let att_amt = att.get_field_amount(sf("sfAmount"));
        let att_was_locking = (att.get_field_u32(sf("sfFlags")) & 0x0000_0001) != 0;

        if att_amt == *sending_amount && att_was_locking == was_locking_chain_send {
            let signer = att.get_account_id(sf("sfAttestationSignerAccount"));
            if let Some(w) = signers_list.get(&signer) {
                weight += *w;
                if att.is_field_present(sf("sfAttestationRewardAccount")) {
                    reward_accounts.push(att.get_account_id(sf("sfAttestationRewardAccount")));
                }
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
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let sle_bridge = match peek_bridge_helper(view, &bridge_spec) {
        Some(sle) => sle,
        None => return Ter::TEC_NO_ENTRY,
    };

    let (signers_list, _quorum, sl_ter) = get_signers_list_and_quorum(view, &sle_bridge);
    if !protocol::is_tes_success(sl_ter) {
        return sl_ter;
    }

    let claim_id = sttx.get_field_u64(sf("sfXChainClaimID"));
    let claim_id_keylet = protocol::xchain_owned_claim_id_keylet_from_bridge(
        Uint160::from_void(bridge_spec.locking_chain_door().data()),
        *bridge_spec.locking_chain_issue().get::<Issue>(),
        Uint160::from_void(bridge_spec.issuing_chain_door().data()),
        *bridge_spec.issuing_chain_issue().get::<Issue>(),
        claim_id,
    );
    let sle_claim_id_arc = match view.peek(claim_id_keylet) {
        Ok(Some(sle)) => sle,
        _ => return Ter::TEC_XCHAIN_NO_CLAIM_ID,
    };

    let mut sle_claim_id = (*sle_claim_id_arc).clone();
    let mut attestations = sle_claim_id
        .get_field_array(sf("sfXChainClaimAttestations"))
        .clone();

    let signer = sttx.get_account_id(sf("sfAttestationSignerAccount"));
    let public_key = match PublicKey::from_slice(&sttx.get_field_vl(sf("sfPublicKey"))) {
        Ok(public_key) => public_key,
        Err(_) => return Ter::TEC_XCHAIN_BAD_PUBLIC_KEY_ACCOUNT_PAIR,
    };
    let signer_key_ter = check_attestation_public_key(view, &signers_list, signer, &public_key);
    if !protocol::is_tes_success(signer_key_ter) {
        return signer_key_ter;
    }

    let mut found = false;
    for att in attestations.iter_mut() {
        if att.get_account_id(sf("sfAttestationSignerAccount")) == signer {
            found = true;
            break;
        }
    }
    if !found {
        let mut new_att = STObject::new(sf("sfXChainClaimAttestation"));
        new_att.set_account_id(sf("sfAttestationSignerAccount"), signer);
        new_att.set_field_vl(sf("sfPublicKey"), &sttx.get_field_vl(sf("sfPublicKey")));
        new_att.set_account_id(
            sf("sfOtherChainSource"),
            sttx.get_account_id(sf("sfOtherChainSource")),
        );
        new_att.set_field_amount(sf("sfAmount"), sttx.get_field_amount(sf("sfAmount")));
        new_att.set_account_id(
            sf("sfAttestationRewardAccount"),
            sttx.get_account_id(sf("sfAttestationRewardAccount")),
        );
        new_att.set_field_u32(
            sf("sfFlags"),
            if (sttx.get_field_u32(sf("sfFlags")) & 0x0000_0001) != 0 {
                1
            } else {
                0
            },
        );
        attestations.push_back(new_att);
    }

    sle_claim_id.set_field_array(sf("sfXChainClaimAttestations"), attestations);
    let _ = view.update(Arc::new(sle_claim_id));

    Ter::TES_SUCCESS
}

pub fn apply_xchain_add_account_create_attestation<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let bridge_spec = sttx.get_field_xchain_bridge(sf("sfXChainBridge"));
    let sle_bridge = match peek_bridge_helper(view, &bridge_spec) {
        Some(sle) => sle,
        None => return Ter::TEC_NO_ENTRY,
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

    let att = attestations::AttestationCreateAccount::from_st_object(sttx);
    if STXChainBridge::dst_chain(att.base.was_locking_chain_send) != dst_chain {
        return Ter::TEC_XCHAIN_WRONG_CHAIN;
    }

    let mut psb = ledger::FlowSandbox::new(view);
    let mut sle_bridge_mut = match peek_bridge_helper(&psb, &bridge_spec) {
        Some(sle) => (*sle).clone(),
        None => return Ter::TEC_INTERNAL,
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
        Err(_) => return Ter::TEC_INTERNAL,
    };
    let create_claim_id = sle_claim_id_arc.is_none();

    if create_claim_id {
        let sle_door = match psb.peek(protocol::account_keylet(Uint160::from_void(
            this_door.data(),
        ))) {
            Ok(Some(sle)) => sle,
            _ => return Ter::TEC_INTERNAL,
        };
        let reserve = psb
            .fees()
            .account_reserve((sle_door.get_field_u32(sf("sfOwnerCount")) + 1) as usize);
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
        let _ = psb.update(Arc::new(updated));
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
        let _ = psb.update(Arc::new(sle_bridge_mut));
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
            _ => return Ter::TEC_DIR_FULL,
        };
        sle_claim_id.set_field_u64(sf("sfOwnerNode"), page);

        let sle_door = match psb.peek(protocol::account_keylet(Uint160::from_void(
            this_door.data(),
        ))) {
            Ok(Some(sle)) => sle,
            _ => return Ter::TEC_INTERNAL,
        };
        let _ = adjust_owner_count(&mut psb, &sle_door, 1);
        let _ = psb.insert(Arc::new(sle_claim_id));
        let _ = psb.update(sle_door);
    }

    let _ = psb.apply();
    Ter::TES_SUCCESS
}
