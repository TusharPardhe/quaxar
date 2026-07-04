//! Transactor bridge — connects the `tx` crate logic to the `app` crate's `ApplyView`.

use basics::math::base_uint::{Uint160, Uint256};
use ledger::views::apply_view::{ApplyView, adjust_owner_count};
use protocol::{
    AccountID, Asset, Keylet, LedgerEntryType, STAmount, STIssue, STLedgerEntry, STObject, STTx,
    Ter, get_field_by_symbol,
};
use std::sync::Arc;
use tx::*;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

// Helper to convert AccountID to Uint160
fn to_160(account: &AccountID) -> Uint160 {
    Uint160::from_void(account.data())
}

fn permissioned_domain_credentials_to_array(
    credentials: Vec<PermissionedDomainCredential<AccountID, Vec<u8>>>,
) -> protocol::STArray {
    let mut array = protocol::STArray::new(sf("sfAcceptedCredentials"));
    array.reserve(credentials.len());
    for credential in credentials {
        let mut entry = STObject::make_inner_object(sf("sfCredential"));
        entry.set_account_id(sf("sfIssuer"), credential.issuer);
        entry.set_field_vl(sf("sfCredentialType"), &credential.credential_type);
        array.push_back(entry);
    }
    array
}

fn update_nft_page_link<V: ApplyView>(
    view: &mut V,
    page: &Arc<STLedgerEntry>,
    field: &'static protocol::SField,
    value: Option<Uint256>,
) -> bool {
    let mut obj = page.clone_as_object();
    match value {
        Some(value) => obj.set_field_h256(field, value),
        None => obj.make_field_absent(field),
    }

    view.update(Arc::new(STLedgerEntry::from_stobject(obj, *page.key())))
        .is_ok()
}

fn repair_nftoken_directory_links<V: ApplyView>(view: &mut V, owner: &AccountID) -> bool {
    let mut did_repair = false;
    let last = protocol::nft_page_max_keylet(to_160(owner));
    let first_key = match view.succ(
        protocol::nft_page_min_keylet(to_160(owner)).key,
        Some(last.key.next()),
    ) {
        Ok(candidate) => candidate.unwrap_or(last.key),
        Err(_) => return false,
    };

    let Some(mut page) = (match view.peek(Keylet::new(LedgerEntryType::NFTokenPage, first_key)) {
        Ok(page) => page,
        Err(_) => return false,
    }) else {
        return false;
    };

    if *page.key() == last.key {
        let next_present = page.is_field_present(sf("sfNextPageMin"));
        let prev_present = page.is_field_present(sf("sfPreviousPageMin"));
        if next_present || prev_present {
            did_repair = true;
            if !update_nft_page_link(view, &page, sf("sfPreviousPageMin"), None)
                || !update_nft_page_link(view, &page, sf("sfNextPageMin"), None)
            {
                return false;
            }
        }
        return did_repair;
    }

    if page.is_field_present(sf("sfPreviousPageMin")) {
        did_repair = true;
        if !update_nft_page_link(view, &page, sf("sfPreviousPageMin"), None) {
            return false;
        }
        let Ok(Some(updated_page)) =
            view.peek(Keylet::new(LedgerEntryType::NFTokenPage, *page.key()))
        else {
            return false;
        };
        page = updated_page;
    }

    let mut next_page = None;
    loop {
        let next_key = match view.succ(page.key().next(), Some(last.key.next())) {
            Ok(candidate) => candidate.unwrap_or(last.key),
            Err(_) => return false,
        };
        let candidate = match view.peek(Keylet::new(LedgerEntryType::NFTokenPage, next_key)) {
            Ok(candidate) => candidate,
            Err(_) => return false,
        };
        let Some(mut candidate) = candidate else {
            break;
        };

        if !page.is_field_present(sf("sfNextPageMin"))
            || page.get_field_h256(sf("sfNextPageMin")) != *candidate.key()
        {
            did_repair = true;
            if !update_nft_page_link(view, &page, sf("sfNextPageMin"), Some(*candidate.key())) {
                return false;
            }
        }

        if !candidate.is_field_present(sf("sfPreviousPageMin"))
            || candidate.get_field_h256(sf("sfPreviousPageMin")) != *page.key()
        {
            did_repair = true;
            if !update_nft_page_link(view, &candidate, sf("sfPreviousPageMin"), Some(*page.key())) {
                return false;
            }
            let Ok(Some(updated_candidate)) =
                view.peek(Keylet::new(LedgerEntryType::NFTokenPage, *candidate.key()))
            else {
                return false;
            };
            candidate = updated_candidate;
        }

        if *candidate.key() == last.key {
            next_page = Some(candidate);
            break;
        }

        page = candidate;
    }

    let Some(next_page) = next_page else {
        did_repair = true;
        let mut repaired_last = STLedgerEntry::new(last);
        repaired_last.set_field_array(sf("sfNFTokens"), page.get_field_array(sf("sfNFTokens")));

        if page.is_field_present(sf("sfPreviousPageMin")) {
            let prev_key = page.get_field_h256(sf("sfPreviousPageMin"));
            repaired_last.set_field_h256(sf("sfPreviousPageMin"), prev_key);

            let Ok(Some(prev_page)) =
                view.peek(Keylet::new(LedgerEntryType::NFTokenPage, prev_key))
            else {
                return false;
            };
            if !update_nft_page_link(view, &prev_page, sf("sfNextPageMin"), Some(last.key)) {
                return false;
            }
        }

        if view.erase(page).is_err() || view.insert(Arc::new(repaired_last)).is_err() {
            return false;
        }
        return did_repair;
    };

    if next_page.is_field_present(sf("sfNextPageMin")) {
        did_repair = true;
        if !update_nft_page_link(view, &next_page, sf("sfNextPageMin"), None) {
            return false;
        }
    }

    did_repair
}

pub fn apply_ledger_state_fix<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let fix_type = if sttx.is_field_present(sf("sfLedgerFixType")) {
        LedgerStateFixType::from(sttx.get_field_u16(sf("sfLedgerFixType")))
    } else {
        LedgerStateFixType::Unknown(0)
    };
    let owner = sttx
        .is_field_present(sf("sfOwner"))
        .then(|| sttx.get_account_id(sf("sfOwner")));
    let book_directory = sttx
        .is_field_present(sf("sfBookDirectory"))
        .then(|| sttx.get_field_h256(sf("sfBookDirectory")));

    let preflight = run_ledger_state_fix_preflight_facts(LedgerStateFixPreflightFacts {
        fix_type,
        owner_present: owner.is_some(),
        book_directory_present: book_directory.is_some(),
        fix_cleanup_3_2_0_enabled: view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_2_0")),
    });
    if preflight != Ter::TES_SUCCESS {
        return preflight;
    }

    let book_dir = book_directory.and_then(|dir_key| {
        view.peek(Keylet::new(LedgerEntryType::DirectoryNode, dir_key))
            .ok()
            .flatten()
    });
    let preclaim = run_ledger_state_fix_preclaim_facts(LedgerStateFixPreclaimFacts {
        fix_type,
        owner_exists: owner.as_ref().is_some_and(|owner| {
            matches!(
                view.peek(protocol::account_keylet(to_160(owner))),
                Ok(Some(_))
            )
        }),
        book_directory_exists: book_dir.is_some(),
        book_directory_has_exchange_rate: book_dir
            .as_ref()
            .is_some_and(|dir| dir.is_field_present(sf("sfExchangeRate"))),
        book_directory_exchange_rate_matches_key: book_dir.as_ref().is_some_and(|dir| {
            dir.is_field_present(sf("sfExchangeRate"))
                && dir.get_field_u64(sf("sfExchangeRate")) == protocol::quality_from_key(*dir.key())
        }),
    });
    if preclaim != Ter::TES_SUCCESS {
        return preclaim;
    }

    match fix_type {
        LedgerStateFixType::NfTokenPageLink => run_ledger_state_fix_do_apply(fix_type, || {
            owner
                .as_ref()
                .is_some_and(|owner| repair_nftoken_directory_links(view, owner))
        }),
        LedgerStateFixType::BookExchangeRate => run_ledger_state_fix_do_apply_with_book(
            fix_type,
            || false,
            || {
                let Some(dir_key) = book_directory else {
                    return false;
                };
                let Ok(Some(dir)) = view.peek(Keylet::new(LedgerEntryType::DirectoryNode, dir_key))
                else {
                    return false;
                };
                let mut obj = dir.clone_as_object();
                obj.set_field_u64(sf("sfExchangeRate"), protocol::quality_from_key(*dir.key()));
                view.update(Arc::new(STLedgerEntry::from_stobject(obj, *dir.key())))
                    .is_ok()
            },
        ),
        LedgerStateFixType::Unknown(_) => {
            run_ledger_state_fix_do_apply_with_book(fix_type, || false, || false)
        }
    }
}

pub struct ViewBackedAccountSetSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> AccountSetDoApplySink for ViewBackedAccountSetSink<'a, V> {
    type AccountId = AccountID;
    fn set_account_txn_id(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            // sfAccountTxnID tracks the last tx that modified this account.
            // Set to zero here; the real value is applied by the outer tx engine.
            obj.set_field_h256(sf("sfAccountTxnID"), Uint256::default());
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_account_txn_id(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(sf("sfAccountTxnID"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_email_hash(&mut self, value: u128) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            let mut buf = [0u8; 16];
            buf.copy_from_slice(&value.to_be_bytes());
            obj.set_field_h128(get_field_by_symbol("sfEmailHash"), buf.into());
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_email_hash(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfEmailHash"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_wallet_locator(&mut self, _value: Vec<u8>) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.set_field_h256(get_field_by_symbol("sfWalletLocator"), Uint256::default());
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_wallet_locator(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfWalletLocator"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_message_key(&mut self, value: Vec<u8>) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.set_stbase(protocol::STBlob::from_buffer(
                get_field_by_symbol("sfMessageKey"),
                basics::buffer::Buffer::from(value.as_slice()),
            ));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_message_key(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfMessageKey"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_domain(&mut self, value: Vec<u8>) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.set_stbase(protocol::STBlob::from_buffer(
                get_field_by_symbol("sfDomain"),
                basics::buffer::Buffer::from(value.as_slice()),
            ));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_domain(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfDomain"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_transfer_rate(&mut self, value: u32) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.set_field_u32(get_field_by_symbol("sfTransferRate"), value);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_transfer_rate(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfTransferRate"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_tick_size(&mut self, value: u8) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.set_field_u8(get_field_by_symbol("sfTickSize"), value);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_tick_size(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfTickSize"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_nftoken_minter(&mut self, value: Self::AccountId) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.set_account_id(get_field_by_symbol("sfNFTokenMinter"), value);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn clear_nftoken_minter(&mut self) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.make_field_absent(get_field_by_symbol("sfNFTokenMinter"));
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_account_flags(&mut self, value: u32) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let mut obj = sle.clone_as_object();
            obj.set_field_u32(get_field_by_symbol("sfFlags"), value);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn update_account(&mut self) {}
}

pub struct ViewBackedAccountDeleteSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub destination: AccountID,
}

impl<'a, V: ApplyView> AccountDeleteDoApplyTailSink for ViewBackedAccountDeleteSink<'a, V> {
    type Amount = STAmount;
    fn source_balance(&mut self) -> Self::Amount {
        let keylet = protocol::account_keylet(to_160(&self.account));
        self.view
            .read(keylet)
            .ok()
            .flatten()
            .map(|sle| sle.get_field_amount(get_field_by_symbol("sfBalance")))
            .unwrap_or_default()
    }
    fn destination_balance(&mut self) -> Self::Amount {
        let keylet = protocol::account_keylet(to_160(&self.destination));
        self.view
            .read(keylet)
            .ok()
            .flatten()
            .map(|sle| sle.get_field_amount(get_field_by_symbol("sfBalance")))
            .unwrap_or_default()
    }
    fn set_source_balance(&mut self, amount: Self::Amount) {
        let keylet = protocol::account_keylet(to_160(&self.account));
        if let Ok(Some(sle)) = self.view.peek(keylet) {
            let mut obj = sle.clone_as_object();
            obj.set_field_amount(get_field_by_symbol("sfBalance"), amount);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn set_destination_balance(&mut self, amount: Self::Amount) {
        let keylet = protocol::account_keylet(to_160(&self.destination));
        if let Ok(Some(sle)) = self.view.peek(keylet) {
            let mut obj = sle.clone_as_object();
            obj.set_field_amount(get_field_by_symbol("sfBalance"), amount);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }
    fn deliver(&mut self, _amount: Self::Amount) {}
    fn owner_dir_exists(&mut self) -> bool {
        let dir_keylet = protocol::owner_dir_keylet(to_160(&self.account));
        self.view.exists(dir_keylet).unwrap_or(false)
    }
    fn empty_dir_delete(&mut self) -> bool {
        let dir_keylet = protocol::owner_dir_keylet(to_160(&self.account));
        if let Ok(Some(sle)) = self.view.read(dir_keylet) {
            let _ = self.view.erase(sle);
        }
        true
    }
    fn destination_password_spent(&mut self) -> bool {
        false
    }
    fn clear_destination_password_spent(&mut self) {}
    fn update_destination(&mut self) {}
    fn erase_source(&mut self) {
        let keylet = protocol::account_keylet(to_160(&self.account));
        if let Ok(Some(sle)) = self.view.read(keylet) {
            let _ = self.view.erase(sle);
        }
    }
}

pub struct ViewBackedDepositPreauthSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> DepositPreauthDoApplyAccountSink for ViewBackedDepositPreauthSink<'a, V> {
    type OwnerNode = u64;
    fn authorize_owner_exists(&mut self) -> bool {
        true
    }
    fn authorize_has_reserve(&mut self) -> bool {
        true
    }
    fn create_authorize_preauth(&mut self) {}
    fn dir_insert_authorize_preauth(&mut self) -> Option<Self::OwnerNode> {
        Some(0)
    }
    fn set_authorize_owner_node(&mut self, _page: Self::OwnerNode) {}
    fn adjust_authorize_owner_count(&mut self) {}
    fn remove_unauthorize_preauth(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
}

impl<'a, V: ApplyView> DepositPreauthDoApplyCredentialSink for ViewBackedDepositPreauthSink<'a, V> {
    type OwnerNode = u64;
    fn authorize_credentials_owner_exists(&mut self) -> bool {
        true
    }
    fn authorize_credentials_has_reserve(&mut self) -> bool {
        true
    }
    fn sort_authorize_credentials(&mut self) {}
    fn create_authorize_credentials_preauth(&mut self) -> bool {
        true
    }
    fn dir_insert_authorize_credentials_preauth(&mut self) -> Option<Self::OwnerNode> {
        Some(0)
    }
    fn set_authorize_credentials_owner_node(&mut self, _page: Self::OwnerNode) {}
    fn adjust_authorize_credentials_owner_count(&mut self) {}
    fn remove_unauthorize_credentials_preauth(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
}

pub struct ViewBackedPermissionedDomainSetSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub tx_sequence: u32,
    pub existing_domain_id: Option<Uint256>,
    staged_domain: Option<STLedgerEntry>,
}

impl<'a, V> ViewBackedPermissionedDomainSetSink<'a, V> {
    pub fn new(
        view: &'a mut V,
        account: AccountID,
        tx_sequence: u32,
        existing_domain_id: Option<Uint256>,
    ) -> Self {
        Self {
            view,
            account,
            tx_sequence,
            existing_domain_id,
            staged_domain: None,
        }
    }
}

impl<'a, V: ApplyView>
    PermissionedDomainSetApplySink<PermissionedDomainCredential<AccountID, Vec<u8>>>
    for ViewBackedPermissionedDomainSetSink<'a, V>
{
    type OwnerNode = u64;

    fn owner_exists(&mut self) -> bool {
        self.view
            .exists(protocol::account_keylet(to_160(&self.account)))
            .unwrap_or(false)
    }

    fn existing_domain_exists(&mut self) -> bool {
        let Some(domain_id) = self.existing_domain_id else {
            return false;
        };

        self.view
            .exists(protocol::permissioned_domain_keylet_from_id(domain_id))
            .unwrap_or(false)
    }

    fn replace_existing_domain_credentials(
        &mut self,
        credentials: Vec<PermissionedDomainCredential<AccountID, Vec<u8>>>,
    ) {
        let Some(domain_id) = self.existing_domain_id else {
            return;
        };

        let keylet = protocol::permissioned_domain_keylet_from_id(domain_id);
        if let Ok(Some(sle)) = self.view.peek(keylet) {
            let mut obj = sle.clone_as_object();
            obj.set_field_array(
                sf("sfAcceptedCredentials"),
                permissioned_domain_credentials_to_array(credentials),
            );
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        }
    }

    fn owner_has_reserve_for_new_domain(&mut self) -> bool {
        let Ok(Some(owner_sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        else {
            return false;
        };

        let balance = owner_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let owner_count = owner_sle.get_field_u32(sf("sfOwnerCount"));
        let reserve = self.view.fees().account_reserve(owner_count as usize + 1) as i64;
        balance >= reserve
    }

    fn stage_new_domain(
        &mut self,
        credentials: Vec<PermissionedDomainCredential<AccountID, Vec<u8>>>,
    ) {
        let keylet = protocol::permissioned_domain_keylet(to_160(&self.account), self.tx_sequence);
        let mut sle = STLedgerEntry::new(keylet);
        sle.set_account_id(sf("sfOwner"), self.account);
        sle.set_field_u32(sf("sfSequence"), self.tx_sequence);
        sle.set_field_array(
            sf("sfAcceptedCredentials"),
            permissioned_domain_credentials_to_array(credentials),
        );
        self.staged_domain = Some(sle);
    }

    fn dir_insert_new_domain(&mut self) -> Option<Self::OwnerNode> {
        let staged_domain = self.staged_domain.as_ref()?;
        ledger::dir_insert(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            *staged_domain.key(),
            &|_| {},
        )
        .ok()
        .flatten()
    }

    fn set_new_domain_owner_node(&mut self, page: Self::OwnerNode) {
        if let Some(staged_domain) = self.staged_domain.as_mut() {
            staged_domain.set_field_u64(sf("sfOwnerNode"), page);
        }
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }

    fn insert_new_domain(&mut self) {
        if let Some(staged_domain) = self.staged_domain.take() {
            let _ = self.view.insert(Arc::new(staged_domain));
        }
    }
}

pub struct ViewBackedPermissionedDomainDeleteSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub domain_id: Uint256,
}

impl<'a, V: ApplyView> PermissionedDomainDeleteLoadedSink
    for ViewBackedPermissionedDomainDeleteSink<'a, V>
{
    fn dir_remove(&mut self) -> bool {
        let keylet = protocol::permissioned_domain_keylet_from_id(self.domain_id);
        let Ok(Some(domain_sle)) = self.view.peek(keylet) else {
            return false;
        };

        ledger::dir_remove(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            domain_sle.get_field_u64(sf("sfOwnerNode")),
            *domain_sle.key(),
            true,
        )
        .unwrap_or(false)
    }

    fn owner_exists_with_nonzero_count(&mut self) -> bool {
        self.view
            .read(protocol::account_keylet(to_160(&self.account)))
            .ok()
            .flatten()
            .map(|sle| sle.get_field_u32(sf("sfOwnerCount")) > 0)
            .unwrap_or(false)
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }

    fn erase_domain(&mut self) {
        let keylet = protocol::permissioned_domain_keylet_from_id(self.domain_id);
        if let Ok(Some(domain_sle)) = self.view.peek(keylet) {
            let _ = self.view.erase(domain_sle);
        }
    }
}

impl<'a, V: ApplyView> PermissionedDomainDeleteApplySink
    for ViewBackedPermissionedDomainDeleteSink<'a, V>
{
    fn loaded_domain_exists(&mut self) -> bool {
        self.view
            .exists(protocol::permissioned_domain_keylet_from_id(self.domain_id))
            .unwrap_or(false)
    }

    fn delete_loaded_domain(&mut self) -> Ter {
        run_permissioned_domain_delete_loaded(self)
    }
}

fn delegate_permissions_to_array(permissions: Vec<u32>) -> protocol::STArray {
    let mut array = protocol::STArray::new(sf("sfPermissions"));
    array.reserve(permissions.len());
    for permission in permissions {
        let mut entry = STObject::make_inner_object(sf("sfPermission"));
        entry.set_field_u32(sf("sfPermissionValue"), permission);
        array.push_back(entry);
    }
    array
}

pub struct ViewBackedDelegateSetSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub authorize: AccountID,
    pub pre_fee_balance_drops: i64,
    staged_delegate: Option<STLedgerEntry>,
}

impl<'a, V> ViewBackedDelegateSetSink<'a, V> {
    pub fn new(
        view: &'a mut V,
        account: AccountID,
        authorize: AccountID,
        pre_fee_balance_drops: i64,
    ) -> Self {
        Self {
            view,
            account,
            authorize,
            pre_fee_balance_drops,
            staged_delegate: None,
        }
    }

    fn keylet(&self) -> Keylet {
        protocol::delegate_keylet(to_160(&self.account), to_160(&self.authorize))
    }
}

impl<'a, V: ApplyView> DelegateSetDeleteSink for ViewBackedDelegateSetSink<'a, V> {
    fn delegate_exists_for_delete(&mut self) -> bool {
        self.view.exists(self.keylet()).unwrap_or(false)
    }

    fn dir_remove_owner(&mut self) -> bool {
        let Ok(Some(delegate_sle)) = self.view.peek(self.keylet()) else {
            return false;
        };

        ledger::dir_remove(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            delegate_sle.get_field_u64(sf("sfOwnerNode")),
            *delegate_sle.key(),
            false,
        )
        .unwrap_or(false)
    }

    fn dir_remove_destination(&mut self) -> Option<bool> {
        let Ok(Some(delegate_sle)) = self.view.peek(self.keylet()) else {
            return None;
        };

        if !delegate_sle.is_field_present(sf("sfDestinationNode")) {
            return None;
        }

        Some(
            ledger::dir_remove(
                self.view,
                &protocol::owner_dir_keylet(to_160(&self.authorize)),
                delegate_sle.get_field_u64(sf("sfDestinationNode")),
                *delegate_sle.key(),
                false,
            )
            .unwrap_or(false),
        )
    }

    fn owner_exists(&mut self) -> bool {
        self.view
            .exists(protocol::account_keylet(to_160(&self.account)))
            .unwrap_or(false)
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }

    fn erase_delegate(&mut self) {
        if let Ok(Some(delegate_sle)) = self.view.peek(self.keylet()) {
            let _ = self.view.erase(delegate_sle);
        }
    }
}

impl<'a, V: ApplyView> DelegateSetApplySink<u32> for ViewBackedDelegateSetSink<'a, V> {
    type OwnerNode = u64;

    fn owner_exists_for_apply(&mut self) -> bool {
        self.owner_exists()
    }

    fn delegate_exists_for_apply(&mut self) -> bool {
        self.delegate_exists_for_delete()
    }

    fn update_existing_permissions(&mut self, permissions: Vec<u32>) {
        if let Ok(Some(delegate_sle)) = self.view.peek(self.keylet()) {
            let mut obj = delegate_sle.clone_as_object();
            obj.set_field_array(
                sf("sfPermissions"),
                delegate_permissions_to_array(permissions),
            );
            let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
                obj,
                *delegate_sle.key(),
            )));
        }
    }

    fn owner_has_reserve_for_create(&mut self) -> bool {
        let Ok(Some(owner_sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        else {
            return false;
        };
        let reserve = self
            .view
            .fees()
            .account_reserve(owner_sle.get_field_u32(sf("sfOwnerCount")) as usize + 1)
            as i64;
        self.pre_fee_balance_drops >= reserve
    }

    fn stage_new_delegate(&mut self, permissions: Vec<u32>) {
        let mut sle = STLedgerEntry::new(self.keylet());
        sle.set_account_id(sf("sfAccount"), self.account);
        sle.set_account_id(sf("sfAuthorize"), self.authorize);
        sle.set_field_array(
            sf("sfPermissions"),
            delegate_permissions_to_array(permissions),
        );
        self.staged_delegate = Some(sle);
    }

    fn dir_insert_owner(&mut self) -> Option<Self::OwnerNode> {
        let staged_delegate = self.staged_delegate.as_ref()?;
        ledger::dir_insert(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            *staged_delegate.key(),
            &|_| {},
        )
        .ok()
        .flatten()
    }

    fn set_owner_node(&mut self, page: Self::OwnerNode) {
        if let Some(staged_delegate) = self.staged_delegate.as_mut() {
            staged_delegate.set_field_u64(sf("sfOwnerNode"), page);
        }
    }

    fn dir_insert_destination(&mut self) -> Option<Self::OwnerNode> {
        let staged_delegate = self.staged_delegate.as_ref()?;
        ledger::dir_insert(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.authorize)),
            *staged_delegate.key(),
            &|_| {},
        )
        .ok()
        .flatten()
    }

    fn set_destination_node(&mut self, page: Self::OwnerNode) {
        if let Some(staged_delegate) = self.staged_delegate.as_mut() {
            staged_delegate.set_field_u64(sf("sfDestinationNode"), page);
        }
    }

    fn insert_new_delegate(&mut self) {
        if let Some(staged_delegate) = self.staged_delegate.take() {
            let _ = self.view.insert(Arc::new(staged_delegate));
        }
    }
}

pub struct ViewBackedNFTokenMintSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> NFTokenMintApplySink for ViewBackedNFTokenMintSink<'a, V> {
    fn mint_nftoken(&mut self, facts: &NFTokenMintApplyFacts) {
        // Build the NFToken object
        let mut token = STObject::new(get_field_by_symbol("sfNFToken"));
        token.set_field_h256(get_field_by_symbol("sfNFTokenID"), facts.nftoken_id);
        if let Some(uri) = &facts.uri {
            token.set_stbase(uri.clone());
        }

        // Use succ-based page lookup matching rippled's nft::insertToken.
        // Find the correct page for this token using the successor search.
        let owner_160 = to_160(&facts.owner);
        let base = protocol::nft_page_min_keylet(owner_160);
        let first = protocol::nft_page_keylet(base, facts.nftoken_id);
        let last = protocol::nft_page_max_keylet(owner_160);

        let page_key = self
            .view
            .succ(first.key, Some(last.key.next()))
            .ok()
            .flatten()
            .unwrap_or(last.key);

        let page_kl = protocol::Keylet::new(protocol::LedgerEntryType::NFTokenPage, page_key);

        if let Ok(Some(page)) = self.view.peek(page_kl) {
            // Page exists — add token to it in sorted order
            let mut tokens: Vec<_> = page
                .get_field_array(get_field_by_symbol("sfNFTokens"))
                .iter()
                .cloned()
                .collect();
            tokens.push(token);
            tokens.sort_by(|a, b| {
                let a_id = a.get_field_h256(get_field_by_symbol("sfNFTokenID"));
                let b_id = b.get_field_h256(get_field_by_symbol("sfNFTokenID"));
                a_id.cmp(&b_id)
            });
            let mut arr = protocol::STArray::new(get_field_by_symbol("sfNFTokens"));
            for t in tokens {
                arr.push_back(t);
            }
            let mut obj = page.clone_as_object();
            obj.set_field_array(get_field_by_symbol("sfNFTokens"), arr);
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *page.key())));
        } else {
            // No page exists — create the MAX page (matching rippled's initial page creation)
            let mut arr = protocol::STArray::new(get_field_by_symbol("sfNFTokens"));
            arr.push_back(token);
            let mut page_sle = STLedgerEntry::new(last);
            page_sle.set_field_array(get_field_by_symbol("sfNFTokens"), arr);
            let _ = self.view.insert(Arc::new(page_sle));
        }
    }
    fn adjust_owner_count(&mut self, delta: i32) {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            let _ = adjust_owner_count(self.view, &sle, delta);
        }
    }
}

pub struct ViewBackedAMMCreateSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub amount1: STAmount,
    pub amount2: STAmount,
    pub trading_fee: u16,
    pub(crate) amm_keylet: Option<Keylet>,
    pub(crate) amm_account: Option<AccountID>,
    pub(crate) lp_tokens: Option<STAmount>,
}

impl<'a, V: ApplyView> AMMCreateApplySink for ViewBackedAMMCreateSink<'a, V> {
    fn create_amm_account(&mut self) -> Ter {
        let amm_keylet = protocol::keylet::amm(self.amount1.asset(), self.amount2.asset());
        if matches!(self.view.read(amm_keylet), Ok(Some(_))) {
            return Ter::TEC_DUPLICATE;
        }

        let pseudo = match ledger::create_pseudo_account(self.view, amm_keylet.key, sf("sfAMMID")) {
            Ok(pseudo) => pseudo,
            Err(err) => return err,
        };
        self.amm_account = Some(pseudo.get_account_id(sf("sfAccount")));
        self.amm_keylet = Some(amm_keylet);
        Ter::TES_SUCCESS
    }

    fn create_amm_entry(&mut self) -> Ter {
        let Some(amm_keylet) = self.amm_keylet else {
            return Ter::TEC_INTERNAL;
        };
        let Some(amm_account) = self.amm_account else {
            return Ter::TEC_INTERNAL;
        };

        let lpt_issue = protocol::amm_lpt_issue_from_assets(
            self.amount1.asset(),
            self.amount2.asset(),
            amm_account,
        );
        let lp_tokens = ledger::amm_helpers::amm_lp_tokens(&self.amount1, &self.amount2, lpt_issue);
        let (asset, asset2) = if self.amount1.asset() <= self.amount2.asset() {
            (self.amount1.asset(), self.amount2.asset())
        } else {
            (self.amount2.asset(), self.amount1.asset())
        };

        let owner_node = match ledger::dir_insert(
            self.view,
            &protocol::owner_dir_keylet(to_160(&amm_account)),
            amm_keylet.key,
            &|_| {},
        ) {
            Ok(Some(node)) => node,
            Ok(None) => return Ter::TEC_DIR_FULL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };

        let mut amm = STLedgerEntry::new(amm_keylet);
        amm.set_account_id(sf("sfAccount"), amm_account);
        amm.set_field_u16(sf("sfTradingFee"), self.trading_fee);
        amm.set_field_amount(sf("sfLPTokenBalance"), lp_tokens.clone());
        amm.set_field_issue(sf("sfAsset"), STIssue::new_with_asset(sf("sfAsset"), asset));
        amm.set_field_issue(
            sf("sfAsset2"),
            STIssue::new_with_asset(sf("sfAsset2"), asset2),
        );
        amm.set_field_u64(sf("sfOwnerNode"), owner_node);
        if self.view.insert(Arc::new(amm)).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }
        self.lp_tokens = Some(lp_tokens);
        Ter::TES_SUCCESS
    }

    fn deposit_initial_liquidity(&mut self) -> Ter {
        let Some(amm_account) = self.amm_account else {
            return Ter::TEC_INTERNAL;
        };
        for amount in [self.amount1.clone(), self.amount2.clone()] {
            let result = send_amm_initial_asset(self.view, &self.account, &amm_account, &amount);
            if result != Ter::TES_SUCCESS {
                return result;
            }
        }
        Ter::TES_SUCCESS
    }

    fn mint_lp_tokens(&mut self) -> Ter {
        let Some(amm_account) = self.amm_account else {
            return Ter::TEC_INTERNAL;
        };
        let Some(lp_tokens) = self.lp_tokens.clone() else {
            return Ter::TEC_INTERNAL;
        };
        ledger::ripple_state_helpers::account_send(
            self.view,
            &amm_account,
            &self.account,
            &lp_tokens,
        )
    }

    fn adjust_owner_count(&mut self, delta: i32) -> Ter {
        if let Ok(Some(sle)) = self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            return adjust_owner_count(self.view, &sle, delta)
                .map(|_| Ter::TES_SUCCESS)
                .unwrap_or(Ter::TEF_BAD_LEDGER);
        }
        Ter::TER_NO_ACCOUNT
    }
}

fn send_amm_initial_asset<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    amm_account: &AccountID,
    amount: &STAmount,
) -> Ter {
    match amount.asset() {
        Asset::MPTIssue(issue) => send_amm_initial_mpt(view, sender, amm_account, amount, issue),
        Asset::Issue(issue) => {
            let result =
                ledger::ripple_state_helpers::account_send(view, sender, amm_account, amount);
            if result != Ter::TES_SUCCESS || issue.native() {
                return result;
            }
            if let Ok(Some(line)) =
                view.peek(protocol::line(*amm_account, issue.issuer(), issue.currency))
            {
                let mut updated = (*line).clone();
                let flags = updated.get_flags() | protocol::lsfAMMNode;
                updated.set_field_u32(sf("sfFlags"), flags);
                return view
                    .update(Arc::new(updated))
                    .map(|_| Ter::TES_SUCCESS)
                    .unwrap_or(Ter::TEF_BAD_LEDGER);
            }
            Ter::TEC_INTERNAL
        }
    }
}

fn send_amm_initial_mpt<V: ApplyView>(
    view: &mut V,
    sender: &AccountID,
    amm_account: &AccountID,
    amount: &STAmount,
    issue: protocol::MPTIssue,
) -> Ter {
    let value = amount.mpt().value();
    if value <= 0 {
        return Ter::TEC_INTERNAL;
    }
    let value = value as u64;
    let mpt_id = issue.mpt_id();
    let issuer = issue.issuer();

    let Some(issuance) = view
        .peek(protocol::mpt_issuance_keylet_from_mptid(mpt_id))
        .ok()
        .flatten()
    else {
        return Ter::TEC_OBJECT_NOT_FOUND;
    };

    if sender == &issuer {
        let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
        let Some(next) = outstanding.checked_add(value) else {
            return Ter::TEC_INTERNAL;
        };
        let mut updated = (*issuance).clone();
        updated.set_field_u64(sf("sfOutstandingAmount"), next);
        if view.update(Arc::new(updated)).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }
    } else {
        let Some(sender_token) = view
            .peek(protocol::mptoken_keylet_from_mptid(mpt_id, to_160(sender)))
            .ok()
            .flatten()
        else {
            return Ter::TEC_NO_AUTH;
        };
        let current = sender_token.get_field_u64(sf("sfMPTAmount"));
        let Some(next) = current.checked_sub(value) else {
            return Ter::TEC_INSUFFICIENT_FUNDS;
        };
        let mut updated = (*sender_token).clone();
        updated.set_field_u64(sf("sfMPTAmount"), next);
        if view.update(Arc::new(updated)).is_err() {
            return Ter::TEF_BAD_LEDGER;
        }
    }

    let flags = protocol::lsfMPTAMM | protocol::lsfMPTAuthorized;
    let result = ledger::mptoken_helpers::create_mp_token(view, mpt_id, amm_account, flags)
        .unwrap_or(Ter::TEF_BAD_LEDGER);
    if result != Ter::TES_SUCCESS && result != Ter::TEC_DUPLICATE {
        return result;
    }

    let Some(amm_token) = view
        .peek(protocol::mptoken_keylet_from_mptid(
            mpt_id,
            to_160(amm_account),
        ))
        .ok()
        .flatten()
    else {
        return Ter::TEC_OBJECT_NOT_FOUND;
    };
    let current = if amm_token.is_field_present(sf("sfMPTAmount")) {
        amm_token.get_field_u64(sf("sfMPTAmount"))
    } else {
        0
    };
    let Some(next) = current.checked_add(value) else {
        return Ter::TEC_INTERNAL;
    };
    let mut updated = (*amm_token).clone();
    updated.set_field_u64(sf("sfMPTAmount"), next);
    view.update(Arc::new(updated))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

pub struct ViewBackedPaymentSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub dst_account: AccountID,
    pub amount: STAmount,
}

impl<'a, V: ApplyView> ViewBackedPaymentSink<'a, V> {
    pub fn new(
        view: &'a mut V,
        account: AccountID,
        dst_account: AccountID,
        amount: STAmount,
    ) -> Self {
        Self {
            view,
            account,
            dst_account,
            amount,
        }
    }
}

pub struct ViewBackedTrustSetSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub limit_amount: STAmount,
    pub tx_flags: u32,
    pub quality_in: Option<u32>,
    pub quality_out: Option<u32>,
}

impl<'a, V: ApplyView> ViewBackedTrustSetSink<'a, V> {
    /// Execute the trust set operation — set limit on the trust line.
    pub fn execute(&mut self) -> Ter {
        let issue = self.limit_amount.issue();
        let dst_account = issue.account;
        let currency = issue.currency;
        let line_keylet = protocol::line(self.account, dst_account, currency);

        if let Ok(Some(sle)) = self.view.peek(line_keylet) {
            // Update existing trust line
            let b_high = self.account > dst_account;
            let limit_field = if !b_high {
                sf("sfLowLimit")
            } else {
                sf("sfHighLimit")
            };
            let mut obj = sle.clone_as_object();
            let mut limit_allow = self.limit_amount.clone();
            limit_allow.set_issuer(self.account);
            obj.set_field_amount(limit_field, limit_allow);
            if let Some(qi) = self.quality_in {
                let qi_field = if !b_high {
                    sf("sfLowQualityIn")
                } else {
                    sf("sfHighQualityIn")
                };
                obj.set_field_u32(qi_field, qi);
            }
            if let Some(qo) = self.quality_out {
                let qo_field = if !b_high {
                    sf("sfLowQualityOut")
                } else {
                    sf("sfHighQualityOut")
                };
                obj.set_field_u32(qo_field, qo);
            }
            let _ = self
                .view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
        } else {
            // Create new trust line
            let mut sle = STLedgerEntry::new(line_keylet);
            let b_high = self.account > dst_account;
            let zero = STAmount::from_xrp_amount(protocol::XRPAmount::new());
            sle.set_field_amount(sf("sfBalance"), zero.clone());
            let mut limit_allow = self.limit_amount.clone();
            limit_allow.set_issuer(self.account);
            if !b_high {
                sle.set_field_amount(sf("sfLowLimit"), limit_allow);
                sle.set_field_amount(sf("sfHighLimit"), zero);
            } else {
                sle.set_field_amount(sf("sfLowLimit"), zero);
                sle.set_field_amount(sf("sfHighLimit"), limit_allow);
            }
            let _ = self.view.insert(Arc::new(sle));
        }
        Ter::TES_SUCCESS
    }
}

pub struct ViewBackedOfferCancelSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub offer_sequence: u32,
}

impl<'a, V: ApplyView> OfferCancelApplySink for ViewBackedOfferCancelSink<'a, V> {
    fn account_exists(&mut self) -> bool {
        true
    }
    fn offer_exists(&mut self) -> bool {
        true
    }
    fn delete_offer(&mut self) -> Ter {
        Ter::TES_SUCCESS
    }
}

pub struct ViewBackedSignerListSetSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

pub struct ViewBackedAMMDepositSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> AMMDepositApplySink for ViewBackedAMMDepositSink<'a, V> {
    fn get_amm_entry(
        &mut self,
        _asset1: &protocol::Asset,
        _asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry> {
        None
    }
    fn update_amm_entry(&mut self, _sle: protocol::STLedgerEntry) {}
    fn deposit_asset(
        &mut self,
        _account: &protocol::AccountID,
        _amount: &protocol::STAmount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
    fn mint_lp_tokens(
        &mut self,
        _account: &protocol::AccountID,
        _amount: &protocol::STAmount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
}

pub struct ViewBackedAMMWithdrawSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> AMMWithdrawApplySink for ViewBackedAMMWithdrawSink<'a, V> {
    fn get_amm_entry(
        &mut self,
        _asset1: &protocol::Asset,
        _asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry> {
        None
    }
    fn update_amm_entry(&mut self, _sle: protocol::STLedgerEntry) {}
    fn withdraw_asset(
        &mut self,
        _account: &protocol::AccountID,
        _amount: &protocol::STAmount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
    fn burn_lp_tokens(
        &mut self,
        _account: &protocol::AccountID,
        _amount: &protocol::STAmount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
}

pub struct ViewBackedAMMVoteSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> AMMVoteApplySink for ViewBackedAMMVoteSink<'a, V> {
    fn get_amm_entry(
        &mut self,
        _asset1: &protocol::Asset,
        _asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry> {
        None
    }
    fn update_amm_entry(&mut self, _sle: protocol::STLedgerEntry) {}
}

fn amm_owner_dir_entries<V: ApplyView>(
    view: &mut V,
    amm_account: &AccountID,
) -> Option<Vec<Uint256>> {
    let owner_dir = protocol::owner_dir_keylet(to_160(amm_account));
    let mut page = 0_u64;
    let mut entries = Vec::new();
    let mut visited = 0_u16;

    loop {
        visited = visited.saturating_add(1);
        if visited > protocol::MAX_DELETABLE_AMM_TRUST_LINES + 4 {
            return None;
        }

        let page_keylet = protocol::page_keylet(owner_dir, page);
        let node = view
            .peek(page_keylet)
            .ok()
            .flatten()
            .or_else(|| view.read(page_keylet).ok().flatten())?;
        entries.extend(node.get_field_v256(sf("sfIndexes")).value().iter().copied());

        let next = node.get_field_u64(sf("sfIndexNext"));
        if next == 0 || next == page {
            break;
        }
        page = next;
    }

    Some(entries)
}

pub(crate) fn delete_empty_amm_owner_entries<V: ApplyView>(
    view: &mut V,
    amm_account: &AccountID,
) -> Ter {
    let Some(entries) = amm_owner_dir_entries(view, amm_account) else {
        return Ter::TEC_INTERNAL;
    };

    let mut trust_lines_deleted = 0_u16;
    for key in entries.iter().copied() {
        let Some(sle) = view.peek(protocol::child_keylet(key)).ok().flatten() else {
            return Ter::TEC_INTERNAL;
        };
        match sle.get_type() {
            LedgerEntryType::AMM | LedgerEntryType::MPToken => {}
            LedgerEntryType::RippleState => {
                if trust_lines_deleted >= protocol::MAX_DELETABLE_AMM_TRUST_LINES {
                    return Ter::TEF_TOO_BIG;
                }
                if sle.get_field_amount(sf("sfBalance")).signum() != 0 {
                    return Ter::TEC_INTERNAL;
                }
                let low = sle.get_field_amount(sf("sfLowLimit")).issue().issuer();
                let high = sle.get_field_amount(sf("sfHighLimit")).issue().issuer();
                let res = crate::state::trust_set::trust_delete(view, &sle, &low, &high);
                if res != Ter::TES_SUCCESS {
                    return res;
                }
                trust_lines_deleted = trust_lines_deleted.saturating_add(1);
            }
            _ => return Ter::TEC_INTERNAL,
        }
    }

    let Some(entries) = amm_owner_dir_entries(view, amm_account) else {
        return Ter::TEC_INTERNAL;
    };
    for key in entries.iter().copied() {
        let Some(sle) = view.peek(protocol::child_keylet(key)).ok().flatten() else {
            return Ter::TEC_INTERNAL;
        };
        match sle.get_type() {
            LedgerEntryType::AMM => {}
            LedgerEntryType::MPToken => {
                let amount = if sle.is_field_present(sf("sfMPTAmount")) {
                    sle.get_field_u64(sf("sfMPTAmount"))
                } else {
                    0
                };
                let locked = if sle.is_field_present(sf("sfLockedAmount")) {
                    sle.get_field_u64(sf("sfLockedAmount"))
                } else {
                    0
                };
                if amount != 0 || locked != 0 {
                    return Ter::TEC_INTERNAL;
                }
                let owner_node = sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = protocol::owner_dir_keylet(to_160(amm_account));
                if ledger::dir_remove(view, &owner_dir, owner_node, *sle.key(), false).is_err() {
                    return Ter::TEF_BAD_LEDGER;
                }
                if view.erase(sle).is_err() {
                    return Ter::TEC_INTERNAL;
                }
            }
            LedgerEntryType::RippleState => return Ter::TEC_INTERNAL,
            _ => return Ter::TEC_INTERNAL,
        }
    }

    Ter::TES_SUCCESS
}

pub struct ViewBackedAMMDeleteSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> AMMDeleteApplySink for ViewBackedAMMDeleteSink<'a, V> {
    fn get_amm_entry(
        &mut self,
        asset1: &protocol::Asset,
        asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry> {
        self.view
            .peek(protocol::keylet::amm(*asset1, *asset2))
            .ok()
            .flatten()
            .map(|sle| (*sle).clone())
    }
    fn delete_amm_entry(&mut self, sle: protocol::STLedgerEntry) -> Ter {
        let amm_account = sle.get_account_id(sf("sfAccount"));
        let owner_dir = protocol::owner_dir_keylet(to_160(&amm_account));
        let owner_node = sle.get_field_u64(sf("sfOwnerNode"));
        match ledger::dir_remove(self.view, &owner_dir, owner_node, *sle.key(), false) {
            Ok(true) => {}
            Ok(false) => return Ter::TEC_INTERNAL,
            Err(_) => return Ter::TEC_INTERNAL,
        }
        self.view
            .erase(Arc::new(sle))
            .map(|_| Ter::TES_SUCCESS)
            .unwrap_or(Ter::TEC_INTERNAL)
    }
    fn delete_amm_account(&mut self, amm_account: &protocol::AccountID) -> Ter {
        let cleanup = delete_empty_amm_owner_entries(self.view, amm_account);
        if cleanup != Ter::TES_SUCCESS {
            return cleanup;
        }
        let account_keylet = protocol::account_keylet(to_160(amm_account));
        let Some(account) = self.view.peek(account_keylet).ok().flatten() else {
            return Ter::TEC_INTERNAL;
        };
        self.view
            .erase(account)
            .map(|_| Ter::TES_SUCCESS)
            .unwrap_or(Ter::TEC_INTERNAL)
    }
}

pub struct ViewBackedClawbackSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

impl<'a, V: ApplyView> ClawbackApplySink for ViewBackedClawbackSink<'a, V> {
    fn clawback_iou(
        &mut self,
        _issuer: &protocol::AccountID,
        _holder: &protocol::AccountID,
        _amount: &protocol::STAmount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
    fn clawback_mpt(
        &mut self,
        _issuer: &protocol::AccountID,
        _holder: &protocol::AccountID,
        _amount: &protocol::STAmount,
    ) -> Ter {
        Ter::TES_SUCCESS
    }
}
