use basics::base_uint::Uint160;
use ledger::{ApplyView, ReadView};
use protocol::{
    AccountID, MPTIssue, STLedgerEntry, STTx, Ter, TxType, get_field_by_symbol, is_tes_success,
    lsfDepositAuth, lsfMPTCanClawback, lsfMPTCanHoldConfidentialBalance, lsfMPTCanTransfer,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn read<V: ReadView>(
    view: &V,
    keylet: protocol::Keylet,
) -> Result<Option<Arc<STLedgerEntry>>, Ter> {
    view.read(keylet).map_err(|_| Ter::TEF_BAD_LEDGER)
}

fn issue(tx: &STTx) -> MPTIssue {
    MPTIssue::new(tx.get_field_h192(sf("sfMPTokenIssuanceID")))
}

fn account_keylet(account: &AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}

fn token_keylet(issue: MPTIssue, account: &AccountID) -> protocol::Keylet {
    protocol::mptoken_keylet_from_mptid(issue.mpt_id(), Uint160::from_void(account.data()))
}

fn optional_vl(tx: &STTx, name: &str) -> Option<Vec<u8>> {
    let field = sf(name);
    tx.is_field_present(field).then(|| tx.get_field_vl(field))
}

fn check_lock<V: ReadView>(view: &V, issue: &MPTIssue, account: &AccountID) -> Ter {
    match ledger::mptoken_helpers::is_frozen_mpt(view, account, issue) {
        Ok(true) => Ter::TEC_LOCKED,
        Ok(false) => Ter::TES_SUCCESS,
        Err(_) => Ter::TEF_BAD_LEDGER,
    }
}

fn check_auth<V: ReadView>(view: &V, issue: &MPTIssue, account: &AccountID) -> Ter {
    match ledger::mptoken_helpers::require_auth_mpt(view, issue, account) {
        Ok(ter) => ter,
        Err(_) => Ter::TEF_BAD_LEDGER,
    }
}

fn check_lock_and_auth<V: ReadView>(view: &V, issue: &MPTIssue, account: &AccountID) -> Ter {
    let frozen = check_lock(view, issue, account);
    if !is_tes_success(frozen) {
        return frozen;
    }
    check_auth(view, issue, account)
}

fn audit_shape(tx: &STTx, issuance: &STLedgerEntry) -> Ter {
    let has_auditor = tx.is_field_present(sf("sfAuditorEncryptedAmount"));
    let requires_auditor = issuance.is_field_present(sf("sfAuditorEncryptionKey"));
    if has_auditor == requires_auditor {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_NO_PERMISSION
    }
}

fn preclaim_convert<V: ReadView>(view: &V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let issue = issue(tx);
    let issuance = match read(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    if !issuance.is_flag(lsfMPTCanHoldConfidentialBalance)
        || !issuance.is_field_present(sf("sfIssuerEncryptionKey"))
    {
        return Ter::TEC_NO_PERMISSION;
    }
    if issuance.get_account_id(sf("sfIssuer")) == account {
        return Ter::TEF_INTERNAL;
    }
    let audit = audit_shape(tx, &issuance);
    if !is_tes_success(audit) {
        return audit;
    }
    let token = match read(view, token_keylet(issue, &account)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    let access = check_lock_and_auth(view, &issue, &account);
    if !is_tes_success(access) {
        return access;
    }
    let amount = tx.get_field_u64(sf("sfMPTAmount"));
    let balance = if token.is_field_present(sf("sfMPTAmount")) {
        token.get_field_u64(sf("sfMPTAmount"))
    } else {
        0
    };
    if balance < amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    let ledger_key = token.is_field_present(sf("sfHolderEncryptionKey"));
    let tx_key = tx.is_field_present(sf("sfHolderEncryptionKey"));
    if !ledger_key && !tx_key {
        return Ter::TEC_NO_PERMISSION;
    }
    if ledger_key && tx_key {
        return Ter::TEC_DUPLICATE;
    }
    let holder_key = if tx_key {
        tx.get_field_vl(sf("sfHolderEncryptionKey"))
    } else {
        token.get_field_vl(sf("sfHolderEncryptionKey"))
    };
    let mut valid = true;
    if tx_key {
        let Some(context) = protocol::confidential_transfer::get_convert_context_hash(
            account.data(),
            issue.mpt_id().data(),
            tx.get_seq_proxy().value(),
        ) else {
            return Ter::TEC_INTERNAL;
        };
        valid &= is_tes_success(protocol::confidential_transfer::verify_schnorr_proof(
            &holder_key,
            &tx.get_field_vl(sf("sfZKProof")),
            &context,
        ));
    }
    valid &= is_tes_success(protocol::confidential_transfer::verify_revealed_amount(
        amount,
        tx.get_field_h256(sf("sfBlindingFactor")).data(),
        &holder_key,
        &tx.get_field_vl(sf("sfHolderEncryptedAmount")),
        &issuance.get_field_vl(sf("sfIssuerEncryptionKey")),
        &tx.get_field_vl(sf("sfIssuerEncryptedAmount")),
        issuance
            .is_field_present(sf("sfAuditorEncryptionKey"))
            .then(|| issuance.get_field_vl(sf("sfAuditorEncryptionKey")))
            .as_deref(),
        optional_vl(tx, "sfAuditorEncryptedAmount").as_deref(),
    ));
    if valid {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_BAD_PROOF
    }
}

fn preclaim_merge<V: ReadView>(view: &V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let issue = issue(tx);
    let issuance = match read(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    if !issuance.is_flag(lsfMPTCanHoldConfidentialBalance) {
        return Ter::TEC_NO_PERMISSION;
    }
    if issuance.get_account_id(sf("sfIssuer")) == account {
        return Ter::TEF_INTERNAL;
    }
    let token = match read(view, token_keylet(issue, &account)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    for field in [
        "sfConfidentialBalanceInbox",
        "sfConfidentialBalanceSpending",
        "sfHolderEncryptionKey",
    ] {
        if !token.is_field_present(sf(field)) {
            return Ter::TEC_NO_PERMISSION;
        }
    }
    check_lock_and_auth(view, &issue, &account)
}

fn preclaim_convert_back<V: ReadView>(view: &V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let issue = issue(tx);
    let issuance = match read(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    if !issuance.is_flag(lsfMPTCanHoldConfidentialBalance)
        || !issuance.is_field_present(sf("sfIssuerEncryptionKey"))
    {
        return Ter::TEC_NO_PERMISSION;
    }
    let audit = audit_shape(tx, &issuance);
    if !is_tes_success(audit) {
        return audit;
    }
    if issuance.get_account_id(sf("sfIssuer")) == account {
        return Ter::TEF_INTERNAL;
    }
    let token = match read(view, token_keylet(issue, &account)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    for field in [
        "sfHolderEncryptionKey",
        "sfConfidentialBalanceSpending",
        "sfIssuerEncryptedBalance",
    ] {
        if !token.is_field_present(sf(field)) {
            return Ter::TEC_NO_PERMISSION;
        }
    }
    if issuance.is_field_present(sf("sfAuditorEncryptionKey"))
        && !token.is_field_present(sf("sfAuditorEncryptedBalance"))
    {
        return Ter::TEF_INTERNAL;
    }
    let amount = tx.get_field_u64(sf("sfMPTAmount"));
    let outstanding = if issuance.is_field_present(sf("sfConfidentialOutstandingAmount")) {
        issuance.get_field_u64(sf("sfConfidentialOutstandingAmount"))
    } else {
        0
    };
    if outstanding < amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    let access = check_lock_and_auth(view, &issue, &account);
    if !is_tes_success(access) {
        return access;
    }
    let Some(context) = protocol::confidential_transfer::get_convert_back_context_hash(
        account.data(),
        issue.mpt_id().data(),
        tx.get_seq_proxy().value(),
        token
            .is_field_present(sf("sfConfidentialBalanceVersion"))
            .then(|| token.get_field_u32(sf("sfConfidentialBalanceVersion")))
            .unwrap_or(0),
    ) else {
        return Ter::TEC_INTERNAL;
    };
    let holder_key = token.get_field_vl(sf("sfHolderEncryptionKey"));
    let revealed = protocol::confidential_transfer::verify_revealed_amount(
        amount,
        tx.get_field_h256(sf("sfBlindingFactor")).data(),
        &holder_key,
        &tx.get_field_vl(sf("sfHolderEncryptedAmount")),
        &issuance.get_field_vl(sf("sfIssuerEncryptionKey")),
        &tx.get_field_vl(sf("sfIssuerEncryptedAmount")),
        issuance
            .is_field_present(sf("sfAuditorEncryptionKey"))
            .then(|| issuance.get_field_vl(sf("sfAuditorEncryptionKey")))
            .as_deref(),
        optional_vl(tx, "sfAuditorEncryptedAmount").as_deref(),
    );
    let proof = protocol::confidential_transfer::verify_convert_back_proof(
        &tx.get_field_vl(sf("sfZKProof")),
        &holder_key,
        &token.get_field_vl(sf("sfConfidentialBalanceSpending")),
        &tx.get_field_vl(sf("sfBalanceCommitment")),
        amount,
        &context,
    );
    if is_tes_success(revealed) && is_tes_success(proof) {
        Ter::TES_SUCCESS
    } else {
        Ter::TEC_BAD_PROOF
    }
}

fn preclaim_send<V: ReadView>(view: &V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    match view.exists(account_keylet(&account)) {
        Ok(true) => {}
        Ok(false) => return Ter::TER_NO_ACCOUNT,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    }
    let destination_account = match read(view, account_keylet(&destination)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_NO_TARGET,
        Err(ter) => return ter,
    };
    if destination_account.is_flag(protocol::lsfRequireDestTag)
        && !tx.is_field_present(sf("sfDestinationTag"))
    {
        return Ter::TEC_DST_TAG_NEEDED;
    }
    let issue = issue(tx);
    let issuance = match read(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    if !issuance.is_flag(lsfMPTCanTransfer) {
        return Ter::TEC_NO_AUTH;
    }
    if !issuance.is_flag(lsfMPTCanHoldConfidentialBalance)
        || !issuance.is_field_present(sf("sfIssuerEncryptionKey"))
        || (issuance.is_field_present(sf("sfTransferFee"))
            && issuance.get_field_u16(sf("sfTransferFee")) > 0)
    {
        return Ter::TEC_NO_PERMISSION;
    }
    let audit = audit_shape(tx, &issuance);
    if !is_tes_success(audit) {
        return audit;
    }
    if issuance.get_account_id(sf("sfIssuer")) == account {
        return Ter::TEF_INTERNAL;
    }
    let sender = match read(view, token_keylet(issue, &account)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    let receiver = match read(view, token_keylet(issue, &destination)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    for (token, fields) in [
        (
            &sender,
            [
                "sfHolderEncryptionKey",
                "sfConfidentialBalanceSpending",
                "sfIssuerEncryptedBalance",
            ],
        ),
        (
            &receiver,
            [
                "sfHolderEncryptionKey",
                "sfConfidentialBalanceInbox",
                "sfIssuerEncryptedBalance",
            ],
        ),
    ] {
        if fields.iter().any(|name| !token.is_field_present(sf(name))) {
            return Ter::TEC_NO_PERMISSION;
        }
    }
    if issuance.is_field_present(sf("sfAuditorEncryptionKey"))
        && (!sender.is_field_present(sf("sfAuditorEncryptedBalance"))
            || !receiver.is_field_present(sf("sfAuditorEncryptedBalance")))
    {
        return Ter::TEF_INTERNAL;
    }
    // rippled checks both freeze states before either authorization state.
    // This ordering is consensus-visible when multiple conditions fail.
    for party in [&account, &destination] {
        let frozen = check_lock(view, &issue, party);
        if !is_tes_success(frozen) {
            return frozen;
        }
    }
    for party in [&account, &destination] {
        let authorized = check_auth(view, &issue, party);
        if !is_tes_success(authorized) {
            return authorized;
        }
    }
    match ledger::credential_helpers::valid(view, tx, &account) {
        Ok(ter) if !is_tes_success(ter) => return ter,
        Ok(_) => {}
        Err(_) => return Ter::TEF_BAD_LEDGER,
    }
    if destination_account.is_flag(lsfDepositAuth) && account != destination {
        let direct = match view.exists(protocol::deposit_preauth_keylet(
            Uint160::from_void(destination.data()),
            Uint160::from_void(account.data()),
        )) {
            Ok(value) => value,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        if !direct {
            if !tx.is_field_present(sf("sfCredentialIDs")) {
                return Ter::TEC_NO_PERMISSION;
            }
            match ledger::credential_helpers::authorized_deposit_preauth(
                view,
                &tx.get_field_v256(sf("sfCredentialIDs")),
                &destination,
            ) {
                Ok(ter) if !is_tes_success(ter) => return ter,
                Ok(_) => {}
                Err(_) => return Ter::TEF_BAD_LEDGER,
            }
        }
    }
    let Some(context) = protocol::confidential_transfer::get_send_context_hash(
        account.data(),
        issue.mpt_id().data(),
        tx.get_seq_proxy().value(),
        destination.data(),
        sender
            .is_field_present(sf("sfConfidentialBalanceVersion"))
            .then(|| sender.get_field_u32(sf("sfConfidentialBalanceVersion")))
            .unwrap_or(0),
    ) else {
        return Ter::TEC_INTERNAL;
    };
    let sender_recipient = (
        sender.get_field_vl(sf("sfHolderEncryptionKey")),
        tx.get_field_vl(sf("sfSenderEncryptedAmount")),
    );
    let destination_recipient = (
        receiver.get_field_vl(sf("sfHolderEncryptionKey")),
        tx.get_field_vl(sf("sfDestinationEncryptedAmount")),
    );
    let issuer_recipient = (
        issuance.get_field_vl(sf("sfIssuerEncryptionKey")),
        tx.get_field_vl(sf("sfIssuerEncryptedAmount")),
    );
    let auditor_recipient = issuance
        .is_field_present(sf("sfAuditorEncryptionKey"))
        .then(|| {
            (
                issuance.get_field_vl(sf("sfAuditorEncryptionKey")),
                tx.get_field_vl(sf("sfAuditorEncryptedAmount")),
            )
        });
    let mut recipients = vec![
        (sender_recipient.0.as_slice(), sender_recipient.1.as_slice()),
        (
            destination_recipient.0.as_slice(),
            destination_recipient.1.as_slice(),
        ),
        (issuer_recipient.0.as_slice(), issuer_recipient.1.as_slice()),
    ];
    if let Some((key, ciphertext)) = auditor_recipient.as_ref() {
        recipients.push((key.as_slice(), ciphertext.as_slice()));
    }
    protocol::confidential_transfer::verify_send_proof(
        &tx.get_field_vl(sf("sfZKProof")),
        &recipients,
        &sender.get_field_vl(sf("sfConfidentialBalanceSpending")),
        &tx.get_field_vl(sf("sfAmountCommitment")),
        &tx.get_field_vl(sf("sfBalanceCommitment")),
        &context,
    )
}

fn preclaim_clawback<V: ReadView>(view: &V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let holder = tx.get_account_id(sf("sfHolder"));
    for (party, missing) in [
        (&account, Ter::TER_NO_ACCOUNT),
        (&holder, Ter::TEC_NO_TARGET),
    ] {
        match view.exists(account_keylet(party)) {
            Ok(true) => {}
            Ok(false) => return missing,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
    }
    let issue = issue(tx);
    let issuance = match read(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    if issuance.get_account_id(sf("sfIssuer")) != account {
        return Ter::TEF_INTERNAL;
    }
    if !issuance.is_field_present(sf("sfIssuerEncryptionKey"))
        || !issuance.is_flag(lsfMPTCanClawback)
        || !issuance.is_flag(lsfMPTCanHoldConfidentialBalance)
    {
        return Ter::TEC_NO_PERMISSION;
    }
    let token = match read(view, token_keylet(issue, &holder)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(ter) => return ter,
    };
    if !token.is_field_present(sf("sfIssuerEncryptedBalance"))
        || !token.is_field_present(sf("sfHolderEncryptionKey"))
    {
        return Ter::TEC_NO_PERMISSION;
    }
    let amount = tx.get_field_u64(sf("sfMPTAmount"));
    let confidential = issuance
        .is_field_present(sf("sfConfidentialOutstandingAmount"))
        .then(|| issuance.get_field_u64(sf("sfConfidentialOutstandingAmount")))
        .unwrap_or(0);
    if amount > confidential || amount > issuance.get_field_u64(sf("sfOutstandingAmount")) {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    let Some(context) = protocol::confidential_transfer::get_clawback_context_hash(
        account.data(),
        issue.mpt_id().data(),
        tx.get_seq_proxy().value(),
        holder.data(),
    ) else {
        return Ter::TEC_INTERNAL;
    };
    protocol::confidential_transfer::verify_clawback_proof(
        amount,
        &tx.get_field_vl(sf("sfZKProof")),
        &issuance.get_field_vl(sf("sfIssuerEncryptionKey")),
        &token.get_field_vl(sf("sfIssuerEncryptedBalance")),
        &context,
    )
}

pub fn preclaim<V: ReadView>(view: &V, tx: &STTx) -> Ter {
    match tx.get_txn_type() {
        TxType::CONFIDENTIAL_MPT_CONVERT => preclaim_convert(view, tx),
        TxType::CONFIDENTIAL_MPT_MERGE_INBOX => preclaim_merge(view, tx),
        TxType::CONFIDENTIAL_MPT_CONVERT_BACK => preclaim_convert_back(view, tx),
        TxType::CONFIDENTIAL_MPT_SEND => preclaim_send(view, tx),
        TxType::CONFIDENTIAL_MPT_CLAWBACK => preclaim_clawback(view, tx),
        _ => Ter::TEM_UNKNOWN,
    }
}

fn peek<V: ApplyView>(view: &mut V, keylet: protocol::Keylet) -> Result<Arc<STLedgerEntry>, Ter> {
    match view.peek(keylet) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(Ter::TEC_INTERNAL),
        Err(_) => Err(Ter::TEF_BAD_LEDGER),
    }
}

fn updated(sle: &Arc<STLedgerEntry>) -> STLedgerEntry {
    STLedgerEntry::from_stobject(sle.clone_as_object(), *sle.key())
}

fn increment_version(token: &mut STLedgerEntry) {
    let current = token
        .is_field_present(sf("sfConfidentialBalanceVersion"))
        .then(|| token.get_field_u32(sf("sfConfidentialBalanceVersion")));
    token.set_field_u32(
        sf("sfConfidentialBalanceVersion"),
        protocol::confidential_transfer::increment_confidential_version(current),
    );
}

fn update<V: ApplyView>(view: &mut V, sle: STLedgerEntry) -> Ter {
    if view.update(Arc::new(sle)).is_ok() {
        Ter::TES_SUCCESS
    } else {
        Ter::TEF_BAD_LEDGER
    }
}

fn apply_convert<V: ApplyView>(view: &mut V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let issue = issue(tx);
    let token_sle = match peek(view, token_keylet(issue, &account)) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let issuance_sle = match peek(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let mut token = updated(&token_sle);
    let mut issuance = updated(&issuance_sle);
    let amount = tx.get_field_u64(sf("sfMPTAmount"));
    let public = token
        .is_field_present(sf("sfMPTAmount"))
        .then(|| token.get_field_u64(sf("sfMPTAmount")))
        .unwrap_or(0);
    let confidential = issuance
        .is_field_present(sf("sfConfidentialOutstandingAmount"))
        .then(|| issuance.get_field_u64(sf("sfConfidentialOutstandingAmount")))
        .unwrap_or(0);
    let Some(public) = public.checked_sub(amount) else {
        return Ter::TEC_INTERNAL;
    };
    let max_amount = protocol::MAX_MP_TOKEN_AMOUNT as u64;
    if confidential > max_amount || amount > max_amount - confidential {
        return Ter::TEC_INTERNAL;
    }
    let confidential = confidential + amount;
    token.set_field_u64(sf("sfMPTAmount"), public);
    issuance.set_field_u64(sf("sfConfidentialOutstandingAmount"), confidential);
    if tx.is_field_present(sf("sfHolderEncryptionKey")) {
        token.set_field_vl(
            sf("sfHolderEncryptionKey"),
            &tx.get_field_vl(sf("sfHolderEncryptionKey")),
        );
    }
    let holder = tx.get_field_vl(sf("sfHolderEncryptedAmount"));
    let issuer = tx.get_field_vl(sf("sfIssuerEncryptedAmount"));
    let auditor = optional_vl(tx, "sfAuditorEncryptedAmount");
    let has_existing = token.is_field_present(sf("sfIssuerEncryptedBalance"))
        && token.is_field_present(sf("sfConfidentialBalanceInbox"))
        && token.is_field_present(sf("sfConfidentialBalanceSpending"));
    let has_none = !token.is_field_present(sf("sfIssuerEncryptedBalance"))
        && !token.is_field_present(sf("sfConfidentialBalanceInbox"))
        && !token.is_field_present(sf("sfConfidentialBalanceSpending"))
        && !token.is_field_present(sf("sfAuditorEncryptedBalance"));
    if has_existing {
        let Some(inbox) = protocol::confidential_transfer::homomorphic_add(
            &holder,
            &token.get_field_vl(sf("sfConfidentialBalanceInbox")),
        ) else {
            return Ter::TEC_INTERNAL;
        };
        let Some(issuer_balance) = protocol::confidential_transfer::homomorphic_add(
            &issuer,
            &token.get_field_vl(sf("sfIssuerEncryptedBalance")),
        ) else {
            return Ter::TEC_INTERNAL;
        };
        token.set_field_vl(sf("sfConfidentialBalanceInbox"), &inbox);
        token.set_field_vl(sf("sfIssuerEncryptedBalance"), &issuer_balance);
        if let Some(auditor) = auditor {
            if !token.is_field_present(sf("sfAuditorEncryptedBalance")) {
                return Ter::TEC_INTERNAL;
            }
            let Some(balance) = protocol::confidential_transfer::homomorphic_add(
                &auditor,
                &token.get_field_vl(sf("sfAuditorEncryptedBalance")),
            ) else {
                return Ter::TEC_INTERNAL;
            };
            token.set_field_vl(sf("sfAuditorEncryptedBalance"), &balance);
        }
    } else if has_none {
        token.set_field_vl(sf("sfConfidentialBalanceInbox"), &holder);
        token.set_field_vl(sf("sfIssuerEncryptedBalance"), &issuer);
        token.set_field_u32(sf("sfConfidentialBalanceVersion"), 0);
        if let Some(auditor) = auditor {
            token.set_field_vl(sf("sfAuditorEncryptedBalance"), &auditor);
        }
        let Some(zero) = protocol::confidential_transfer::encrypt_canonical_zero_amount(
            &token.get_field_vl(sf("sfHolderEncryptionKey")),
            account.data(),
            issue.mpt_id().data(),
        ) else {
            return Ter::TEC_INTERNAL;
        };
        token.set_field_vl(sf("sfConfidentialBalanceSpending"), &zero);
    } else {
        return Ter::TEC_INTERNAL;
    }
    let result = update(view, issuance);
    if !is_tes_success(result) {
        return result;
    }
    update(view, token)
}

fn apply_merge<V: ApplyView>(view: &mut V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let issue = issue(tx);
    let token_sle = match peek(view, token_keylet(issue, &account)) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let mut token = updated(&token_sle);
    let Some(spending) = protocol::confidential_transfer::homomorphic_add(
        &token.get_field_vl(sf("sfConfidentialBalanceSpending")),
        &token.get_field_vl(sf("sfConfidentialBalanceInbox")),
    ) else {
        return Ter::TEC_INTERNAL;
    };
    let Some(zero) = protocol::confidential_transfer::encrypt_canonical_zero_amount(
        &token.get_field_vl(sf("sfHolderEncryptionKey")),
        account.data(),
        issue.mpt_id().data(),
    ) else {
        return Ter::TEC_INTERNAL;
    };
    token.set_field_vl(sf("sfConfidentialBalanceSpending"), &spending);
    token.set_field_vl(sf("sfConfidentialBalanceInbox"), &zero);
    increment_version(&mut token);
    update(view, token)
}

fn apply_convert_back<V: ApplyView>(view: &mut V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let issue = issue(tx);
    let token_sle = match peek(view, token_keylet(issue, &account)) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let issuance_sle = match peek(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let mut token = updated(&token_sle);
    let mut issuance = updated(&issuance_sle);
    let amount = tx.get_field_u64(sf("sfMPTAmount"));
    let public = token
        .is_field_present(sf("sfMPTAmount"))
        .then(|| token.get_field_u64(sf("sfMPTAmount")))
        .unwrap_or(0);
    let outstanding = issuance
        .is_field_present(sf("sfConfidentialOutstandingAmount"))
        .then(|| issuance.get_field_u64(sf("sfConfidentialOutstandingAmount")))
        .unwrap_or(0);
    let max_amount = protocol::MAX_MP_TOKEN_AMOUNT as u64;
    if public > max_amount || amount > max_amount - public {
        return Ter::TEC_INTERNAL;
    }
    let public = public + amount;
    let Some(outstanding) = outstanding.checked_sub(amount) else {
        return Ter::TEC_INTERNAL;
    };
    token.set_field_u64(sf("sfMPTAmount"), public);
    issuance.set_field_u64(sf("sfConfidentialOutstandingAmount"), outstanding);
    for (ledger_field, tx_field) in [
        ("sfConfidentialBalanceSpending", "sfHolderEncryptedAmount"),
        ("sfIssuerEncryptedBalance", "sfIssuerEncryptedAmount"),
        ("sfAuditorEncryptedBalance", "sfAuditorEncryptedAmount"),
    ] {
        if tx.is_field_present(sf(tx_field)) {
            let Some(value) = protocol::confidential_transfer::homomorphic_subtract(
                &token.get_field_vl(sf(ledger_field)),
                &tx.get_field_vl(sf(tx_field)),
            ) else {
                return Ter::TEC_INTERNAL;
            };
            token.set_field_vl(sf(ledger_field), &value);
        }
    }
    increment_version(&mut token);
    let result = update(view, issuance);
    if !is_tes_success(result) {
        return result;
    }
    update(view, token)
}

fn apply_send<V: ApplyView>(view: &mut V, tx: &STTx) -> Ter {
    let account = tx.get_account_id(sf("sfAccount"));
    let destination = tx.get_account_id(sf("sfDestination"));
    let issue = issue(tx);
    let sender_sle = match peek(view, token_keylet(issue, &account)) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let receiver_sle = match peek(view, token_keylet(issue, &destination)) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let issuance = match read(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };
    let _destination_account = match read(view, account_keylet(&destination)) {
        Ok(Some(value)) => value,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(ter) => return ter,
    };
    match ledger::credential_helpers::cleanup_expired_credentials(tx, view) {
        Ok(ter) if !is_tes_success(ter) => return ter,
        Ok(_) => {}
        Err(_) => return Ter::TEF_BAD_LEDGER,
    }
    let mut sender = updated(&sender_sle);
    let mut receiver = updated(&receiver_sle);
    for (ledger_field, tx_field) in [
        ("sfConfidentialBalanceSpending", "sfSenderEncryptedAmount"),
        ("sfIssuerEncryptedBalance", "sfIssuerEncryptedAmount"),
        ("sfAuditorEncryptedBalance", "sfAuditorEncryptedAmount"),
    ] {
        if tx.is_field_present(sf(tx_field)) {
            let Some(value) = protocol::confidential_transfer::homomorphic_subtract(
                &sender.get_field_vl(sf(ledger_field)),
                &tx.get_field_vl(sf(tx_field)),
            ) else {
                return Ter::TEC_INTERNAL;
            };
            sender.set_field_vl(sf(ledger_field), &value);
        }
    }
    let challenge = tx.get_field_vl(sf("sfZKProof"));
    let randomness = &challenge[..protocol::confidential_transfer::EC_BLINDING_FACTOR_LENGTH];
    for (ledger_field, tx_field, key_field) in [
        (
            "sfConfidentialBalanceInbox",
            "sfDestinationEncryptedAmount",
            "sfHolderEncryptionKey",
        ),
        (
            "sfIssuerEncryptedBalance",
            "sfIssuerEncryptedAmount",
            "sfIssuerEncryptionKey",
        ),
        (
            "sfAuditorEncryptedBalance",
            "sfAuditorEncryptedAmount",
            "sfAuditorEncryptionKey",
        ),
    ] {
        if tx.is_field_present(sf(tx_field)) {
            let key_owner = if key_field == "sfHolderEncryptionKey" {
                &receiver
            } else {
                issuance.as_ref()
            };
            let Some(rerandomized) = protocol::confidential_transfer::rerandomize_ciphertext(
                &tx.get_field_vl(sf(tx_field)),
                &key_owner.get_field_vl(sf(key_field)),
                randomness,
            ) else {
                return Ter::TEC_INTERNAL;
            };
            let Some(value) = protocol::confidential_transfer::homomorphic_add(
                &receiver.get_field_vl(sf(ledger_field)),
                &rerandomized,
            ) else {
                return Ter::TEC_INTERNAL;
            };
            receiver.set_field_vl(sf(ledger_field), &value);
        }
    }
    increment_version(&mut sender);
    let result = update(view, sender);
    if !is_tes_success(result) {
        return result;
    }
    update(view, receiver)
}

fn apply_clawback<V: ApplyView>(view: &mut V, tx: &STTx) -> Ter {
    let holder = tx.get_account_id(sf("sfHolder"));
    let issue = issue(tx);
    let issuance_sle = match peek(
        view,
        protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
    ) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let token_sle = match peek(view, token_keylet(issue, &holder)) {
        Ok(value) => value,
        Err(ter) => return ter,
    };
    let mut issuance = updated(&issuance_sle);
    let mut token = updated(&token_sle);
    for (balance_field, key_field) in [
        ("sfConfidentialBalanceInbox", "sfHolderEncryptionKey"),
        ("sfConfidentialBalanceSpending", "sfHolderEncryptionKey"),
        ("sfIssuerEncryptedBalance", "sfIssuerEncryptionKey"),
        ("sfAuditorEncryptedBalance", "sfAuditorEncryptionKey"),
    ] {
        if balance_field == "sfAuditorEncryptedBalance"
            && !token.is_field_present(sf(balance_field))
        {
            continue;
        }
        let key_owner = if key_field == "sfHolderEncryptionKey" {
            &token
        } else {
            &issuance
        };
        if !key_owner.is_field_present(sf(key_field)) {
            return Ter::TEC_INTERNAL;
        }
        let Some(zero) = protocol::confidential_transfer::encrypt_canonical_zero_amount(
            &key_owner.get_field_vl(sf(key_field)),
            holder.data(),
            issue.mpt_id().data(),
        ) else {
            return Ter::TEC_INTERNAL;
        };
        token.set_field_vl(sf(balance_field), &zero);
    }
    increment_version(&mut token);
    let amount = tx.get_field_u64(sf("sfMPTAmount"));
    for field in ["sfConfidentialOutstandingAmount", "sfOutstandingAmount"] {
        let Some(value) = issuance.get_field_u64(sf(field)).checked_sub(amount) else {
            return Ter::TEC_INTERNAL;
        };
        issuance.set_field_u64(sf(field), value);
    }
    let result = update(view, token);
    if !is_tes_success(result) {
        return result;
    }
    update(view, issuance)
}

pub fn apply<V: ApplyView>(view: &mut V, tx: &STTx) -> Ter {
    match tx.get_txn_type() {
        TxType::CONFIDENTIAL_MPT_CONVERT => apply_convert(view, tx),
        TxType::CONFIDENTIAL_MPT_MERGE_INBOX => apply_merge(view, tx),
        TxType::CONFIDENTIAL_MPT_CONVERT_BACK => apply_convert_back(view, tx),
        TxType::CONFIDENTIAL_MPT_SEND => apply_send(view, tx),
        TxType::CONFIDENTIAL_MPT_CLAWBACK => apply_clawback(view, tx),
        _ => Ter::TEM_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::base_uint::Uint256;
    use ledger::{Ledger, RawView, Sandbox};
    use protocol::{ApplyFlags, STAmount};

    fn account_entry(account: AccountID) -> STLedgerEntry {
        let mut sle = STLedgerEntry::new(account_keylet(&account));
        sle.set_account_id(sf("sfAccount"), account);
        sle.set_field_amount(sf("sfBalance"), STAmount::new_native(100_000_000, false));
        sle.set_field_u32(sf("sfSequence"), 1);
        sle.set_field_u32(sf("sfOwnerCount"), 1);
        sle.set_field_u32(sf("sfFlags"), 0);
        sle.set_field_h256(sf("sfPreviousTxnID"), Uint256::zero());
        sle.set_field_u32(sf("sfPreviousTxnLgrSeq"), 0);
        sle
    }

    fn issuance_entry(
        id: protocol::MPTID,
        issuer: AccountID,
        issuer_key: &[u8],
        outstanding: u64,
    ) -> STLedgerEntry {
        let keylet = protocol::mpt_issuance_keylet_from_mptid(id);
        let mut sle = STLedgerEntry::new(keylet);
        sle.set_account_id(sf("sfIssuer"), issuer);
        sle.set_field_u32(sf("sfSequence"), 1);
        sle.set_field_u64(sf("sfOwnerNode"), 0);
        sle.set_field_u64(sf("sfOutstandingAmount"), outstanding);
        sle.set_field_u64(sf("sfConfidentialOutstandingAmount"), 0);
        sle.set_field_u32(
            sf("sfFlags"),
            lsfMPTCanHoldConfidentialBalance | lsfMPTCanTransfer | lsfMPTCanClawback,
        );
        sle.set_field_vl(sf("sfIssuerEncryptionKey"), issuer_key);
        sle.set_field_h256(sf("sfPreviousTxnID"), Uint256::zero());
        sle.set_field_u32(sf("sfPreviousTxnLgrSeq"), 0);
        sle
    }

    fn token_entry(
        id: protocol::MPTID,
        account: AccountID,
        amount: u64,
        holder_key: Option<&[u8]>,
    ) -> STLedgerEntry {
        let mut sle = STLedgerEntry::new(token_keylet(MPTIssue::new(id), &account));
        sle.set_account_id(sf("sfAccount"), account);
        sle.set_field_h192(sf("sfMPTokenIssuanceID"), id);
        sle.set_field_u64(sf("sfMPTAmount"), amount);
        sle.set_field_u64(sf("sfOwnerNode"), 0);
        sle.set_field_h256(sf("sfPreviousTxnID"), Uint256::zero());
        sle.set_field_u32(sf("sfPreviousTxnLgrSeq"), 0);
        if let Some(holder_key) = holder_key {
            sle.set_field_vl(sf("sfHolderEncryptionKey"), holder_key);
        }
        sle
    }

    fn encrypted(amount: u64, key: &[u8], scalar: u8) -> Vec<u8> {
        protocol::confidential_transfer::encrypt_amount(amount, key, &[scalar; 32]).unwrap()
    }

    fn commit_apply(
        ledger: &mut Ledger,
        label: &str,
        tx: &STTx,
    ) -> (protocol::TxMeta, ledger::SHAMapHash) {
        let base = Arc::new(ledger.clone());
        let mut sandbox = Sandbox::new(base, ApplyFlags::NONE);
        assert_eq!(apply(&mut sandbox, tx), Ter::TES_SUCCESS, "{label}");
        let meta = sandbox
            .table()
            .to_tx_meta(tx.get_transaction_id(), ledger.seq(), None);
        sandbox.apply(ledger).unwrap();
        ledger.flush_state_map_to_store();
        (meta, ledger.state_map_mut().hash())
    }

    #[test]
    fn pinned_confidential_apply_flow_has_exact_state_metadata_and_root_deltas() {
        if !protocol::confidential_transfer::confidential_crypto_available() {
            eprintln!("skipping confidential state/root fixture: official ABI unavailable");
            return;
        }
        let issuer = AccountID::from_array([0x11; 20]);
        let holder = AccountID::from_array([0x22; 20]);
        let destination = AccountID::from_array([0x33; 20]);
        let id = protocol::make_mpt_id(1, issuer);
        let issue = MPTIssue::new(id);
        let holder_key = secp256k1::PublicKey::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_byte_array([1; 32]).unwrap(),
        )
        .serialize();
        let issuer_key = secp256k1::PublicKey::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_byte_array([2; 32]).unwrap(),
        )
        .serialize();
        let destination_key = secp256k1::PublicKey::from_secret_key(
            &secp256k1::Secp256k1::new(),
            &secp256k1::SecretKey::from_byte_array([3; 32]).unwrap(),
        )
        .serialize();

        let mut ledger = Ledger::from_ledger_seq_and_close_time(1, 0, false);
        for account in [issuer, holder, destination] {
            ledger.raw_insert(Arc::new(account_entry(account))).unwrap();
        }
        ledger
            .raw_insert(Arc::new(issuance_entry(id, issuer, &issuer_key, 100)))
            .unwrap();
        ledger
            .raw_insert(Arc::new(token_entry(id, holder, 100, None)))
            .unwrap();
        let mut destination_token = token_entry(id, destination, 0, Some(&destination_key));
        let destination_zero = protocol::confidential_transfer::encrypt_canonical_zero_amount(
            &destination_key,
            destination.data(),
            id.data(),
        )
        .unwrap();
        let destination_issuer_zero =
            protocol::confidential_transfer::encrypt_canonical_zero_amount(
                &issuer_key,
                destination.data(),
                id.data(),
            )
            .unwrap();
        destination_token.set_field_vl(sf("sfConfidentialBalanceInbox"), &destination_zero);
        destination_token.set_field_vl(sf("sfConfidentialBalanceSpending"), &destination_zero);
        destination_token.set_field_vl(sf("sfIssuerEncryptedBalance"), &destination_issuer_zero);
        destination_token.set_field_u32(sf("sfConfidentialBalanceVersion"), 0);
        ledger.raw_insert(Arc::new(destination_token)).unwrap();
        ledger.flush_state_map_to_store();
        let initial_root = ledger.state_map_mut().hash();

        let convert_holder = encrypted(60, &holder_key, 4);
        let convert_issuer = encrypted(60, &issuer_key, 4);
        let convert = STTx::new(TxType::CONFIDENTIAL_MPT_CONVERT, |tx| {
            tx.set_account_id(sf("sfAccount"), holder);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), id);
            tx.set_field_u64(sf("sfMPTAmount"), 60);
            tx.set_field_vl(sf("sfHolderEncryptionKey"), &holder_key);
            tx.set_field_vl(sf("sfHolderEncryptedAmount"), &convert_holder);
            tx.set_field_vl(sf("sfIssuerEncryptedAmount"), &convert_issuer);
        });
        let (convert_meta, convert_root) = commit_apply(&mut ledger, "convert", &convert);
        assert_eq!(convert_meta.get_nodes().len(), 2);
        let token = ledger.read(token_keylet(issue, &holder)).unwrap().unwrap();
        assert_eq!(token.get_field_u64(sf("sfMPTAmount")), 40);
        assert_eq!(
            token.get_field_vl(sf("sfConfidentialBalanceInbox")),
            convert_holder
        );
        assert_eq!(token.get_field_u32(sf("sfConfidentialBalanceVersion")), 0);
        assert_ne!(convert_root, initial_root);

        let merge = STTx::new(TxType::CONFIDENTIAL_MPT_MERGE_INBOX, |tx| {
            tx.set_account_id(sf("sfAccount"), holder);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), id);
        });
        let expected_merged_spending = protocol::confidential_transfer::homomorphic_add(
            &token.get_field_vl(sf("sfConfidentialBalanceSpending")),
            &token.get_field_vl(sf("sfConfidentialBalanceInbox")),
        )
        .unwrap();
        let (merge_meta, merge_root) = commit_apply(&mut ledger, "merge-holder", &merge);
        assert_eq!(merge_meta.get_nodes().len(), 1);
        let token = ledger.read(token_keylet(issue, &holder)).unwrap().unwrap();
        assert_eq!(
            token.get_field_vl(sf("sfConfidentialBalanceSpending")),
            expected_merged_spending
        );
        assert_eq!(token.get_field_u32(sf("sfConfidentialBalanceVersion")), 1);
        assert_ne!(merge_root, convert_root);

        let back_holder = encrypted(10, &holder_key, 5);
        let back_issuer = encrypted(10, &issuer_key, 5);
        let convert_back = STTx::new(TxType::CONFIDENTIAL_MPT_CONVERT_BACK, |tx| {
            tx.set_account_id(sf("sfAccount"), holder);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), id);
            tx.set_field_u64(sf("sfMPTAmount"), 10);
            tx.set_field_vl(sf("sfHolderEncryptedAmount"), &back_holder);
            tx.set_field_vl(sf("sfIssuerEncryptedAmount"), &back_issuer);
        });
        let (back_meta, back_root) = commit_apply(&mut ledger, "convert-back", &convert_back);
        assert_eq!(back_meta.get_nodes().len(), 2);
        let token = ledger.read(token_keylet(issue, &holder)).unwrap().unwrap();
        assert_eq!(token.get_field_u64(sf("sfMPTAmount")), 50);
        assert_eq!(token.get_field_u32(sf("sfConfidentialBalanceVersion")), 2);
        assert_eq!(
            ledger
                .read(protocol::mpt_issuance_keylet_from_mptid(id))
                .unwrap()
                .unwrap()
                .get_field_u64(sf("sfConfidentialOutstandingAmount")),
            50
        );
        assert_ne!(back_root, merge_root);

        let send_holder = encrypted(20, &holder_key, 6);
        let send_destination = encrypted(20, &destination_key, 6);
        let send_issuer = encrypted(20, &issuer_key, 6);
        let send = STTx::new(TxType::CONFIDENTIAL_MPT_SEND, |tx| {
            tx.set_account_id(sf("sfAccount"), holder);
            tx.set_account_id(sf("sfDestination"), destination);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), id);
            tx.set_field_vl(sf("sfSenderEncryptedAmount"), &send_holder);
            tx.set_field_vl(sf("sfDestinationEncryptedAmount"), &send_destination);
            tx.set_field_vl(sf("sfIssuerEncryptedAmount"), &send_issuer);
            tx.set_field_vl(
                sf("sfZKProof"),
                &vec![7; protocol::confidential_transfer::EC_SEND_PROOF_LENGTH],
            );
        });
        let (send_meta, send_root) = commit_apply(&mut ledger, "send", &send);
        assert_eq!(send_meta.get_nodes().len(), 2);
        assert_eq!(
            ledger
                .read(token_keylet(issue, &holder))
                .unwrap()
                .unwrap()
                .get_field_u32(sf("sfConfidentialBalanceVersion")),
            3
        );
        assert_ne!(send_root, back_root);

        let merge_destination = STTx::new(TxType::CONFIDENTIAL_MPT_MERGE_INBOX, |tx| {
            tx.set_account_id(sf("sfAccount"), destination);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), id);
        });
        let (_, destination_merge_root) =
            commit_apply(&mut ledger, "merge-destination", &merge_destination);
        let clawback = STTx::new(TxType::CONFIDENTIAL_MPT_CLAWBACK, |tx| {
            tx.set_account_id(sf("sfAccount"), issuer);
            tx.set_account_id(sf("sfHolder"), destination);
            tx.set_field_h192(sf("sfMPTokenIssuanceID"), id);
            tx.set_field_u64(sf("sfMPTAmount"), 20);
        });
        let (clawback_meta, clawback_root) = commit_apply(&mut ledger, "clawback", &clawback);
        assert_eq!(clawback_meta.get_nodes().len(), 2);
        let destination_token = ledger
            .read(token_keylet(issue, &destination))
            .unwrap()
            .unwrap();
        assert_eq!(
            destination_token.get_field_u32(sf("sfConfidentialBalanceVersion")),
            2
        );
        assert_eq!(
            destination_token.get_field_vl(sf("sfConfidentialBalanceInbox")),
            destination_zero
        );
        assert_eq!(
            destination_token.get_field_vl(sf("sfIssuerEncryptedBalance")),
            destination_issuer_zero
        );
        let issuance = ledger
            .read(protocol::mpt_issuance_keylet_from_mptid(id))
            .unwrap()
            .unwrap();
        assert_eq!(
            issuance.get_field_u64(sf("sfConfidentialOutstandingAmount")),
            30
        );
        assert_eq!(issuance.get_field_u64(sf("sfOutstandingAmount")), 80);
        assert_ne!(clawback_root, destination_merge_root);
    }
}
