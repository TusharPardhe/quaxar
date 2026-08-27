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

fn repair_nftoken_directory_links<V: ApplyView>(
    view: &mut V,
    owner: &AccountID,
) -> Result<bool, ledger::ViewError> {
    ledger::nftoken_helpers::repair_nftoken_directory_links(view, owner)
}

fn nft_repair_result_to_ter(result: Result<bool, ledger::ViewError>) -> Ter {
    match result {
        Ok(true) => Ter::TES_SUCCESS,
        Ok(false) => Ter::TEC_FAILED_PROCESSING,
        Err(_) => Ter::TEF_BAD_LEDGER,
    }
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

    let book_dir = match book_directory {
        Some(dir_key) => match view.peek(Keylet::new(LedgerEntryType::DirectoryNode, dir_key)) {
            Ok(dir) => dir,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        },
        None => None,
    };
    let owner_exists = match owner.as_ref() {
        Some(owner) => match view.peek(protocol::account_keylet(to_160(owner))) {
            Ok(owner) => owner.is_some(),
            Err(_) => return Ter::TEF_BAD_LEDGER,
        },
        None => false,
    };
    let preclaim = run_ledger_state_fix_preclaim_facts(LedgerStateFixPreclaimFacts {
        fix_type,
        owner_exists,
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
        LedgerStateFixType::NfTokenPageLink => {
            let Some(owner) = owner.as_ref() else {
                return Ter::TEC_INTERNAL;
            };
            nft_repair_result_to_ter(repair_nftoken_directory_links(view, owner))
        }
        LedgerStateFixType::BookExchangeRate => {
            let Some(dir_key) = book_directory else {
                return Ter::TEC_INTERNAL;
            };
            let dir = match view.peek(Keylet::new(LedgerEntryType::DirectoryNode, dir_key)) {
                Ok(Some(dir)) => dir,
                Ok(None) => return Ter::TEC_INTERNAL,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let mut obj = dir.clone_as_object();
            obj.set_field_u64(sf("sfExchangeRate"), protocol::quality_from_key(*dir.key()));
            match view.update(Arc::new(STLedgerEntry::from_stobject(obj, *dir.key()))) {
                Ok(()) => Ter::TES_SUCCESS,
                Err(_) => Ter::TEF_BAD_LEDGER,
            }
        }
        LedgerStateFixType::Unknown(_) => {
            run_ledger_state_fix_do_apply_with_book(fix_type, || false, || false)
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
    pub failure: Option<Ter>,
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
            failure: None,
        }
    }
}

impl<'a, V: ApplyView>
    PermissionedDomainSetApplySink<PermissionedDomainCredential<AccountID, Vec<u8>>>
    for ViewBackedPermissionedDomainSetSink<'a, V>
{
    type OwnerNode = u64;

    fn owner_exists(&mut self) -> bool {
        match self
            .view
            .exists(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(exists) => exists,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }
        }
    }

    fn existing_domain_exists(&mut self) -> bool {
        let Some(domain_id) = self.existing_domain_id else {
            return false;
        };

        match self
            .view
            .exists(protocol::permissioned_domain_keylet_from_id(domain_id))
        {
            Ok(exists) => exists,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }
        }
    }

    fn replace_existing_domain_credentials(
        &mut self,
        credentials: Vec<PermissionedDomainCredential<AccountID, Vec<u8>>>,
    ) {
        let Some(domain_id) = self.existing_domain_id else {
            return;
        };

        let keylet = protocol::permissioned_domain_keylet_from_id(domain_id);
        let sle = match self.view.peek(keylet) {
            Ok(Some(sle)) => sle,
            Ok(None) => {
                self.failure = Some(Ter::TEF_INTERNAL);
                return;
            }
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return;
            }
        };
        let mut obj = sle.clone_as_object();
        obj.set_field_array(
            sf("sfAcceptedCredentials"),
            permissioned_domain_credentials_to_array(credentials),
        );
        if self
            .view
            .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
            .is_err()
        {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }

    fn owner_has_reserve_for_new_domain(&mut self) -> bool {
        let owner_sle = match self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(Some(sle)) => sle,
            Ok(None) => {
                self.failure = Some(Ter::TEF_INTERNAL);
                return false;
            }
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return false;
            }
        };

        let balance = owner_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        // Pinned accountReserve includes Sponsor-era sponsored/sponsoring
        // owner counts and account-base reserve responsibility. A plain
        // OwnerCount+1 calculation overcharges sponsored accounts and
        // undercharges accounts sponsoring other objects/accounts.
        let Ok(reserve) = i64::try_from(ledger::effective_account_reserve(
            self.view.fees(),
            &owner_sle,
            1,
            0,
        )) else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
            return false;
        };
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
        match ledger::dir_insert(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            *staged_domain.key(),
            &ledger::describe_owner_dir(self.account),
        ) {
            Ok(page) => page,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                None
            }
        }
    }

    fn set_new_domain_owner_node(&mut self, page: Self::OwnerNode) {
        if let Some(staged_domain) = self.staged_domain.as_mut() {
            staged_domain.set_field_u64(sf("sfOwnerNode"), page);
        }
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        match self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(Some(sle)) => {
                if adjust_owner_count(self.view, &sle, delta).is_err() {
                    self.failure = Some(Ter::TEF_BAD_LEDGER);
                }
            }
            Ok(None) => self.failure = Some(Ter::TEF_INTERNAL),
            Err(_) => self.failure = Some(Ter::TEF_BAD_LEDGER),
        }
    }

    fn insert_new_domain(&mut self) {
        if let Some(staged_domain) = self.staged_domain.take() {
            if self.view.insert(Arc::new(staged_domain)).is_err() {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
        }
    }
}

pub struct ViewBackedPermissionedDomainDeleteSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub domain_id: Uint256,
    pub failure: Option<Ter>,
}

impl<'a, V: ApplyView> PermissionedDomainDeleteLoadedSink
    for ViewBackedPermissionedDomainDeleteSink<'a, V>
{
    fn dir_remove(&mut self) -> bool {
        let keylet = protocol::permissioned_domain_keylet_from_id(self.domain_id);
        let domain_sle = match self.view.peek(keylet) {
            Ok(Some(sle)) => sle,
            Ok(None) => {
                self.failure = Some(Ter::TEF_INTERNAL);
                return false;
            }
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return false;
            }
        };

        match ledger::dir_remove(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            domain_sle.get_field_u64(sf("sfOwnerNode")),
            *domain_sle.key(),
            true,
        ) {
            Ok(removed) => removed,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }
        }
    }

    fn owner_exists_with_nonzero_count(&mut self) -> bool {
        match self
            .view
            .read(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(Some(sle)) => sle.get_field_u32(sf("sfOwnerCount")) > 0,
            Ok(None) => {
                self.failure = Some(Ter::TEF_INTERNAL);
                false
            }
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }
        }
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        match self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(Some(sle)) => {
                if adjust_owner_count(self.view, &sle, delta).is_err() {
                    self.failure = Some(Ter::TEF_BAD_LEDGER);
                }
            }
            Ok(None) => self.failure = Some(Ter::TEF_INTERNAL),
            Err(_) => self.failure = Some(Ter::TEF_BAD_LEDGER),
        }
    }

    fn erase_domain(&mut self) {
        let keylet = protocol::permissioned_domain_keylet_from_id(self.domain_id);
        match self.view.peek(keylet) {
            Ok(Some(domain_sle)) => {
                if self.view.erase(domain_sle).is_err() {
                    self.failure = Some(Ter::TEF_BAD_LEDGER);
                }
            }
            Ok(None) => self.failure = Some(Ter::TEF_INTERNAL),
            Err(_) => self.failure = Some(Ter::TEF_BAD_LEDGER),
        }
    }
}

impl<'a, V: ApplyView> PermissionedDomainDeleteApplySink
    for ViewBackedPermissionedDomainDeleteSink<'a, V>
{
    fn loaded_domain_exists(&mut self) -> bool {
        match self
            .view
            .exists(protocol::permissioned_domain_keylet_from_id(self.domain_id))
        {
            Ok(exists) => exists,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }
        }
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
    pub reserve_sponsor: Option<Arc<STLedgerEntry>>,
    pub failure: Option<Ter>,
    staged_delegate: Option<STLedgerEntry>,
}

impl<'a, V> ViewBackedDelegateSetSink<'a, V> {
    pub fn new(
        view: &'a mut V,
        account: AccountID,
        authorize: AccountID,
        pre_fee_balance_drops: i64,
        reserve_sponsor: Option<Arc<STLedgerEntry>>,
    ) -> Self {
        Self {
            view,
            account,
            authorize,
            pre_fee_balance_drops,
            reserve_sponsor,
            failure: None,
            staged_delegate: None,
        }
    }

    fn keylet(&self) -> Keylet {
        protocol::delegate_keylet(to_160(&self.account), to_160(&self.authorize))
    }
}

impl<'a, V: ApplyView> DelegateSetDeleteSink for ViewBackedDelegateSetSink<'a, V> {
    fn delegate_exists_for_delete(&mut self) -> bool {
        match self.view.exists(self.keylet()) {
            Ok(exists) => exists,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }
        }
    }

    fn dir_remove_owner(&mut self) -> bool {
        let delegate_sle = match self.view.peek(self.keylet()) {
            Ok(Some(sle)) => sle,
            Ok(None) => return false,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return false;
            }
        };

        ledger::dir_remove(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            delegate_sle.get_field_u64(sf("sfOwnerNode")),
            *delegate_sle.key(),
            false,
        )
        .unwrap_or_else(|_| {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
            false
        })
    }

    fn dir_remove_destination(&mut self) -> Option<bool> {
        let delegate_sle = match self.view.peek(self.keylet()) {
            Ok(Some(sle)) => sle,
            Ok(None) => return None,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return Some(false);
            }
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
            .unwrap_or_else(|_| {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }),
        )
    }

    fn owner_exists(&mut self) -> bool {
        match self
            .view
            .exists(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(exists) => exists,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                false
            }
        }
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        let sle = match self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(Some(sle)) => sle,
            Ok(None) => {
                self.failure = Some(Ter::TEC_INTERNAL);
                return;
            }
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return;
            }
        };
        let result = if delta < 0 {
            match self.view.peek(self.keylet()) {
                Ok(Some(delegate_sle)) => ledger::decrease_owner_count_for_object(
                    self.view,
                    &sle,
                    &delegate_sle,
                    delta.unsigned_abs(),
                ),
                Ok(None) => {
                    self.failure = Some(Ter::TEC_INTERNAL);
                    return;
                }
                Err(_) => {
                    self.failure = Some(Ter::TEF_BAD_LEDGER);
                    return;
                }
            }
        } else if delta == 1 {
            ledger::increase_owner_count_for_object(self.view, &sle, self.reserve_sponsor.as_ref())
        } else {
            adjust_owner_count(self.view, &sle, delta)
        };
        if result.is_err() {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
        }
    }

    fn erase_delegate(&mut self) {
        match self.view.peek(self.keylet()) {
            Ok(Some(delegate_sle)) => {
                if self.view.erase(delegate_sle).is_err() {
                    self.failure = Some(Ter::TEF_BAD_LEDGER);
                }
            }
            Ok(None) => self.failure = Some(Ter::TEC_INTERNAL),
            Err(_) => self.failure = Some(Ter::TEF_BAD_LEDGER),
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
        match self.view.peek(self.keylet()) {
            Ok(Some(delegate_sle)) => {
                let mut obj = delegate_sle.clone_as_object();
                obj.set_field_array(
                    sf("sfPermissions"),
                    delegate_permissions_to_array(permissions),
                );
                if self
                    .view
                    .update(Arc::new(STLedgerEntry::from_stobject(
                        obj,
                        *delegate_sle.key(),
                    )))
                    .is_err()
                {
                    self.failure = Some(Ter::TEF_BAD_LEDGER);
                }
            }
            Ok(None) => self.failure = Some(Ter::TEC_INTERNAL),
            Err(_) => self.failure = Some(Ter::TEF_BAD_LEDGER),
        }
    }

    fn owner_has_reserve_for_create(&mut self) -> bool {
        let owner_sle = match self
            .view
            .peek(protocol::account_keylet(to_160(&self.account)))
        {
            Ok(Some(sle)) => sle,
            Ok(None) => return false,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                return false;
            }
        };
        let reserve_sle = self
            .reserve_sponsor
            .as_deref()
            .unwrap_or(owner_sle.as_ref());
        let reserve = ledger::effective_account_reserve(self.view.fees(), reserve_sle, 1, 0);
        let Ok(reserve) = i64::try_from(reserve) else {
            self.failure = Some(Ter::TEF_BAD_LEDGER);
            return false;
        };
        let balance = self
            .reserve_sponsor
            .as_ref()
            .map_or(self.pre_fee_balance_drops, |sponsor| {
                sponsor.get_field_amount(sf("sfBalance")).xrp().drops()
            });
        balance >= reserve
    }

    fn stage_new_delegate(&mut self, permissions: Vec<u32>) {
        let mut sle = STLedgerEntry::new(self.keylet());
        sle.set_account_id(sf("sfAccount"), self.account);
        sle.set_account_id(sf("sfAuthorize"), self.authorize);
        sle.set_field_array(
            sf("sfPermissions"),
            delegate_permissions_to_array(permissions),
        );
        if let Some(sponsor) = self.reserve_sponsor.as_ref() {
            sle.set_account_id(sf("sfSponsor"), sponsor.get_account_id(sf("sfAccount")));
        }
        self.staged_delegate = Some(sle);
    }

    fn dir_insert_owner(&mut self) -> Option<Self::OwnerNode> {
        let staged_delegate = self.staged_delegate.as_ref()?;
        match ledger::dir_insert(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.account)),
            *staged_delegate.key(),
            &ledger::describe_owner_dir(self.account),
        ) {
            Ok(page) => page,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                None
            }
        }
    }

    fn set_owner_node(&mut self, page: Self::OwnerNode) {
        if let Some(staged_delegate) = self.staged_delegate.as_mut() {
            staged_delegate.set_field_u64(sf("sfOwnerNode"), page);
        }
    }

    fn dir_insert_destination(&mut self) -> Option<Self::OwnerNode> {
        let staged_delegate = self.staged_delegate.as_ref()?;
        match ledger::dir_insert(
            self.view,
            &protocol::owner_dir_keylet(to_160(&self.authorize)),
            *staged_delegate.key(),
            &ledger::describe_owner_dir(self.authorize),
        ) {
            Ok(page) => page,
            Err(_) => {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
                None
            }
        }
    }

    fn set_destination_node(&mut self, page: Self::OwnerNode) {
        if let Some(staged_delegate) = self.staged_delegate.as_mut() {
            staged_delegate.set_field_u64(sf("sfDestinationNode"), page);
        }
    }

    fn insert_new_delegate(&mut self) {
        if let Some(staged_delegate) = self.staged_delegate.take() {
            if self.view.insert(Arc::new(staged_delegate)).is_err() {
                self.failure = Some(Ter::TEF_BAD_LEDGER);
            }
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
        match self.view.read(amm_keylet) {
            Ok(Some(_)) => return Ter::TEC_DUPLICATE,
            Ok(None) => {}
            Err(_) => return Ter::TEF_BAD_LEDGER,
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
            &ledger::describe_owner_dir(amm_account),
        ) {
            Ok(Some(node)) => node,
            Ok(None) => return Ter::TEC_DIR_FULL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };

        let mut amm = STLedgerEntry::new(amm_keylet);
        amm.set_account_id(sf("sfAccount"), amm_account);
        if self.trading_fee != 0 {
            amm.set_field_u16(sf("sfTradingFee"), self.trading_fee);
        }
        amm.set_field_amount(sf("sfLPTokenBalance"), lp_tokens.clone());
        amm.set_field_issue(sf("sfAsset"), STIssue::new_with_asset(sf("sfAsset"), asset));
        amm.set_field_issue(
            sf("sfAsset2"),
            STIssue::new_with_asset(sf("sfAsset2"), asset2),
        );
        amm.set_field_u64(sf("sfOwnerNode"), owner_node);

        let mut vote_slots = protocol::STArray::new(sf("sfVoteSlots"));
        let mut vote = STObject::make_inner_object(sf("sfVoteEntry"));
        vote.set_account_id(sf("sfAccount"), self.account);
        vote.set_field_u32(sf("sfVoteWeight"), protocol::VOTE_WEIGHT_SCALE_FACTOR);
        if self.trading_fee != 0 {
            vote.set_field_u16(sf("sfTradingFee"), self.trading_fee);
        }
        vote_slots.push_back(vote);
        amm.set_field_array(sf("sfVoteSlots"), vote_slots);

        let mut auction_slot = STObject::make_inner_object(sf("sfAuctionSlot"));
        auction_slot.set_account_id(sf("sfAccount"), self.account);
        auction_slot.set_field_u32(
            sf("sfExpiration"),
            self.view
                .header()
                .parent_close_time
                .saturating_add(protocol::TOTAL_TIME_SLOT_SECS),
        );
        auction_slot.set_field_amount(sf("sfPrice"), lp_tokens.zeroed());
        let discounted_fee =
            self.trading_fee / protocol::AUCTION_SLOT_DISCOUNTED_FEE_FRACTION as u16;
        if discounted_fee != 0 {
            auction_slot.set_field_u16(sf("sfDiscountedFee"), discounted_fee);
        }
        amm.set_field_object(sf("sfAuctionSlot"), auction_slot);

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
            // Pinned AMMCreate uses accountSend(...,
            // WaiveTransferFee::Yes): a third-party creator deposits the exact
            // amount even when the IOU issuer configured a transfer rate.
            let result = ledger::ripple_state_helpers::account_send_waive_transfer_fee(
                view,
                sender,
                amm_account,
                amount,
            );
            if result != Ter::TES_SUCCESS || issue.native() {
                return result;
            }
            let line = match view.peek(protocol::line(*amm_account, issue.issuer(), issue.currency))
            {
                Ok(Some(line)) => line,
                Ok(None) => return Ter::TEC_INTERNAL,
                Err(_) => return Ter::TEF_BAD_LEDGER,
            };
            let mut updated = (*line).clone();
            let flags = updated.get_flags() | protocol::lsfAMMNode;
            updated.set_field_u32(sf("sfFlags"), flags);
            view.update(Arc::new(updated))
                .map(|_| Ter::TES_SUCCESS)
                .unwrap_or(Ter::TEF_BAD_LEDGER)
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

    let issuance = match view.peek(protocol::mpt_issuance_keylet_from_mptid(mpt_id)) {
        Ok(Some(issuance)) => issuance,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(_) => return Ter::TEF_BAD_LEDGER,
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
        let sender_token =
            match view.peek(protocol::mptoken_keylet_from_mptid(mpt_id, to_160(sender))) {
                Ok(Some(token)) => token,
                Ok(None) => return Ter::TEC_NO_AUTH,
                Err(_) => return Ter::TEF_BAD_LEDGER,
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

    let amm_token = match view.peek(protocol::mptoken_keylet_from_mptid(
        mpt_id,
        to_160(amm_account),
    )) {
        Ok(Some(token)) => token,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(_) => return Ter::TEF_BAD_LEDGER,
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

pub struct ViewBackedSignerListSetSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
}

fn amm_owner_dir_entries<V: ApplyView>(
    view: &mut V,
    amm_account: &AccountID,
) -> Result<Vec<Uint256>, Ter> {
    let owner_dir = protocol::owner_dir_keylet(to_160(amm_account));
    let mut page = 0_u64;
    let mut entries = Vec::new();
    let mut visited = 0_u16;

    loop {
        visited = visited.saturating_add(1);
        if visited > protocol::MAX_DELETABLE_AMM_TRUST_LINES + 4 {
            return Err(Ter::TEC_INTERNAL);
        }

        let page_keylet = protocol::page_keylet(owner_dir, page);
        let node = match view.peek(page_keylet) {
            Ok(Some(node)) => node,
            Ok(None) => return Err(Ter::TEC_INTERNAL),
            Err(_) => return Err(Ter::TEF_BAD_LEDGER),
        };
        entries.extend(node.get_field_v256(sf("sfIndexes")).value().iter().copied());

        let next = node.get_field_u64(sf("sfIndexNext"));
        if next == 0 || next == page {
            break;
        }
        page = next;
    }

    Ok(entries)
}

fn require_amm_owner_child(
    result: Result<Option<Arc<protocol::STLedgerEntry>>, ledger::ViewError>,
) -> Result<Arc<protocol::STLedgerEntry>, Ter> {
    // cleanupOnAccountDelete treats both a missing indexed child and a view
    // failure as malformed ledger state (tefBAD_LEDGER). Neither condition is
    // a recoverable AMM semantic failure.
    result
        .map_err(|_| Ter::TEF_BAD_LEDGER)?
        .ok_or(Ter::TEF_BAD_LEDGER)
}

pub(crate) fn delete_empty_amm_owner_entries<V: ApplyView>(
    view: &mut V,
    amm_account: &AccountID,
) -> Ter {
    let entries = match amm_owner_dir_entries(view, amm_account) {
        Ok(entries) => entries,
        Err(ter) => return ter,
    };

    // cleanupOnAccountDelete applies its bound to every owner-directory
    // entry visited, including entries which the AMM cleanup deliberately
    // skips.  Keep that exact accounting so repeated AMMDelete transactions
    // make the same bounded progress as rippled.
    let mut entries_visited = 0_u16;
    for key in entries.iter().copied() {
        entries_visited = entries_visited.saturating_add(1);
        if entries_visited > protocol::MAX_DELETABLE_AMM_TRUST_LINES {
            return Ter::TEC_INCOMPLETE;
        }
        let sle = match require_amm_owner_child(view.peek(protocol::child_keylet(key))) {
            Ok(sle) => sle,
            Err(ter) => return ter,
        };
        match sle.get_type() {
            LedgerEntryType::AMM | LedgerEntryType::MPToken => {}
            LedgerEntryType::RippleState => {
                if sle.get_field_amount(sf("sfBalance")).signum() != 0 {
                    return Ter::TEC_INTERNAL;
                }
                let low = sle.get_field_amount(sf("sfLowLimit")).issue().issuer();
                let high = sle.get_field_amount(sf("sfHighLimit")).issue().issuer();
                let res = crate::state::trust_set::trust_delete(view, &sle, &low, &high);
                if res != Ter::TES_SUCCESS {
                    return res;
                }
            }
            _ => return Ter::TEC_INTERNAL,
        }
    }

    let entries = match amm_owner_dir_entries(view, amm_account) {
        Ok(entries) => entries,
        Err(ter) => return ter,
    };
    for key in entries.iter().copied() {
        let sle = match require_amm_owner_child(view.peek(protocol::child_keylet(key))) {
            Ok(sle) => sle,
            Err(ter) => return ter,
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
                match ledger::dir_remove(view, &owner_dir, owner_node, *sle.key(), false) {
                    Ok(true) => {}
                    Ok(false) => return Ter::TEC_INTERNAL,
                    Err(_) => return Ter::TEF_BAD_LEDGER,
                }
                if view.erase(sle).is_err() {
                    return Ter::TEF_BAD_LEDGER;
                }
            }
            LedgerEntryType::RippleState => return Ter::TEC_INTERNAL,
            _ => return Ter::TEC_INTERNAL,
        }
    }

    Ter::TES_SUCCESS
}

/// Delete an empty AMM pseudo-account after its bounded owner cleanup.
///
/// `tecINCOMPLETE` deliberately leaves the AMM object and account root in
/// place. The outer transactor persistence path retains only the deleted
/// trust lines, allowing a later AMMDelete/AMMWithdraw to continue safely.
pub(crate) fn delete_amm_account<V: ApplyView>(
    view: &mut V,
    amm_sle: &Arc<protocol::STLedgerEntry>,
) -> Ter {
    let amm_account = amm_sle.get_account_id(sf("sfAccount"));
    let cleanup = delete_empty_amm_owner_entries(view, &amm_account);
    if cleanup != Ter::TES_SUCCESS {
        return cleanup;
    }

    let owner_dir = protocol::owner_dir_keylet(to_160(&amm_account));
    let owner_node = amm_sle.get_field_u64(sf("sfOwnerNode"));
    match ledger::dir_remove(view, &owner_dir, owner_node, *amm_sle.key(), false) {
        Ok(true) => {}
        Ok(false) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    }

    let account_keylet = protocol::account_keylet(to_160(&amm_account));
    let account = match view.peek(account_keylet) {
        Ok(Some(account)) => account,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    if view.erase(amm_sle.clone()).is_err() || view.erase(account).is_err() {
        return Ter::TEF_BAD_LEDGER;
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
    ) -> Result<Option<protocol::STLedgerEntry>, Ter> {
        self.view
            .peek(protocol::keylet::amm(*asset1, *asset2))
            .map(|entry| entry.map(|sle| (*sle).clone()))
            .map_err(|_| Ter::TEF_BAD_LEDGER)
    }
    fn delete_amm_entry(&mut self, sle: protocol::STLedgerEntry) -> Ter {
        let amm_account = sle.get_account_id(sf("sfAccount"));
        let owner_dir = protocol::owner_dir_keylet(to_160(&amm_account));
        let owner_node = sle.get_field_u64(sf("sfOwnerNode"));
        match ledger::dir_remove(self.view, &owner_dir, owner_node, *sle.key(), false) {
            Ok(true) => {}
            Ok(false) => return Ter::TEC_INTERNAL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
        self.view
            .erase(Arc::new(sle))
            .map(|_| Ter::TES_SUCCESS)
            .unwrap_or(Ter::TEF_BAD_LEDGER)
    }
    fn delete_amm_account(&mut self, amm_account: &protocol::AccountID) -> Ter {
        let cleanup = delete_empty_amm_owner_entries(self.view, amm_account);
        if cleanup != Ter::TES_SUCCESS {
            return cleanup;
        }
        let account_keylet = protocol::account_keylet(to_160(amm_account));
        let account = match self.view.peek(account_keylet) {
            Ok(Some(account)) => account,
            Ok(None) => return Ter::TEC_INTERNAL,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        };
        self.view
            .erase(account)
            .map(|_| Ter::TES_SUCCESS)
            .unwrap_or(Ter::TEF_BAD_LEDGER)
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

#[cfg(test)]
mod parity_tests {
    use super::{nft_repair_result_to_ter, require_amm_owner_child};
    use protocol::Ter;

    #[test]
    fn ledger_state_fix_distinguishes_no_repair_from_storage_failure() {
        assert_eq!(nft_repair_result_to_ter(Ok(true)), Ter::TES_SUCCESS);
        assert_eq!(
            nft_repair_result_to_ter(Ok(false)),
            Ter::TEC_FAILED_PROCESSING
        );
        assert_eq!(
            nft_repair_result_to_ter(Err(ledger::ViewError::Conversion(
                "injected NFTokenPage read failure".into(),
            ))),
            Ter::TEF_BAD_LEDGER
        );
    }

    #[test]
    fn amm_cleanup_missing_or_faulting_directory_child_is_bad_ledger() {
        assert_eq!(
            require_amm_owner_child(Ok(None)).expect_err("missing indexed child must fail hard"),
            Ter::TEF_BAD_LEDGER
        );
        assert_eq!(
            require_amm_owner_child(Err(ledger::ViewError::Conversion(
                "fault-injected AMM child read".into(),
            )))
            .expect_err("storage failure must fail hard"),
            Ter::TEF_BAD_LEDGER
        );
    }
}
