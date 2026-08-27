//! Directory helpers matching reference `xrpl::directory` namespace and
//! `ApplyView::dirAdd`/`dirRemove`/`dirAppend`/`dirInsert`/`dirDelete`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{AccountID, Keylet, STLedgerEntry, STObject, STVector256};

use crate::read_view::ViewError;
use crate::views::apply_view::ApplyView;

pub const DIR_NODE_MAX_ENTRIES: usize = 32;

/// reference: `kDirNodeMaxPages` in `xrpl/protocol/Protocol.h`. Legacy cap on
/// the number of pages a directory chain may grow to; made obsolete by the
/// `fixDirectoryLimit` amendment.
pub const DIR_NODE_MAX_PAGES: u64 = 262_144;

/// Describe an owner-directory page exactly as rippled's
/// `describeOwnerDir`: every page created for the directory carries the
/// account in `sfOwner`.
pub fn describe_owner_dir(account: AccountID) -> impl Fn(&mut STObject) {
    move |directory| directory.set_account_id(sf("sfOwner"), account)
}

fn sf(name: &str) -> &'static protocol::SField {
    protocol::get_field_by_symbol(name)
}

fn page_kl(directory: &Keylet, page: u64) -> Keylet {
    protocol::page_keylet(*directory, page)
}

fn sle_update(sle: &Arc<STLedgerEntry>, mutate: impl FnOnce(&mut STObject)) -> Arc<STLedgerEntry> {
    let mut obj = sle.clone_as_object();
    mutate(&mut obj);
    Arc::new(STLedgerEntry::from_stobject(obj, *sle.key()))
}

fn v256_to_vec(v: &STVector256) -> Vec<Uint256> {
    v.value().to_vec()
}

fn vec_to_v256(v: Vec<Uint256>) -> STVector256 {
    STVector256::from_values(sf("sfIndexes"), v)
}

// ---------------------------------------------------------------------------
// Internal directory namespace helpers
// ---------------------------------------------------------------------------

fn create_root(
    view: &mut dyn ApplyView,
    directory: &Keylet,
    key: Uint256,
    describe: &dyn Fn(&mut STObject),
) -> Result<u64, ViewError> {
    let mut root = STLedgerEntry::new(*directory);
    root.set_field_h256(sf("sfRootIndex"), directory.key);
    describe(&mut root);
    root.set_field_v256(sf("sfIndexes"), vec_to_v256(vec![key]));
    view.insert(Arc::new(root))?;
    Ok(0)
}

fn find_previous_page(
    view: &mut dyn ApplyView,
    directory: &Keylet,
    start: &Arc<STLedgerEntry>,
) -> Result<(u64, Arc<STLedgerEntry>, Vec<Uint256>), ViewError> {
    let page = start.get_field_u64(sf("sfIndexPrevious"));
    let node = if page != 0 {
        let keylet = page_kl(directory, page);
        view.peek(keylet)?.ok_or_else(|| {
            ViewError::Conversion("Directory chain: root back-pointer broken.".into())
        })?
    } else {
        Arc::clone(start)
    };
    let indexes = v256_to_vec(&node.get_field_v256(sf("sfIndexes")));
    Ok((page, node, indexes))
}

fn insert_key(
    view: &mut dyn ApplyView,
    node: &Arc<STLedgerEntry>,
    page: u64,
    preserve_order: bool,
    indexes: &mut Vec<Uint256>,
    key: Uint256,
) -> Result<u64, ViewError> {
    if preserve_order {
        if indexes.contains(&key) {
            return Err(ViewError::Conversion("dirInsert: double insertion".into()));
        }
        indexes.push(key);
    } else {
        indexes.sort();
        match indexes.binary_search(&key) {
            Ok(_) => return Err(ViewError::Conversion("dirInsert: double insertion".into())),
            Err(pos) => indexes.insert(pos, key),
        }
    }
    view.raw_replace(sle_update(node, |obj| {
        obj.set_field_v256(sf("sfIndexes"), vec_to_v256(indexes.clone()));
    }))?;
    Ok(page)
}

/// reference: `ApplyView::dirAdd` -> `directory::insertPage` in
/// `xrpl/ledger/ApplyView.cpp` -- before the `fixDirectoryLimit` amendment,
/// directory chains were capped at `kDirNodeMaxPages` pages. Pure predicate
/// so the boundary condition is unit-testable without an `ApplyView`.
fn directory_page_limit_exceeded(new_page: u64, fix_directory_limit_enabled: bool) -> bool {
    !fix_directory_limit_enabled && new_page >= DIR_NODE_MAX_PAGES
}

fn insert_page(
    view: &mut dyn ApplyView,
    page: u64,
    node: &Arc<STLedgerEntry>,
    _next_page: u64,
    next: &Arc<STLedgerEntry>,
    key: Uint256,
    directory: &Keylet,
    describe: &dyn Fn(&mut STObject),
) -> Result<Option<u64>, ViewError> {
    let new_page = page.wrapping_add(1);
    if new_page == 0 {
        return Ok(None);
    }
    let fix_directory_limit_enabled = view
        .rules()
        .enabled(&protocol::feature_id("fixDirectoryLimit"));
    if directory_page_limit_exceeded(new_page, fix_directory_limit_enabled) {
        return Ok(None);
    }

    if node.key() == next.key() {
        // The first overflow page links the root to itself. Both link fields
        // must be written from one SLE snapshot: issuing two replacements from
        // the same original root drops whichever field the second write does
        // not set.
        view.raw_replace(sle_update(node, |obj| {
            obj.set_field_u64(sf("sfIndexNext"), new_page);
            obj.set_field_u64(sf("sfIndexPrevious"), new_page);
        }))?;
    } else {
        view.raw_replace(sle_update(node, |obj| {
            obj.set_field_u64(sf("sfIndexNext"), new_page);
        }))?;

        view.raw_replace(sle_update(next, |obj| {
            obj.set_field_u64(sf("sfIndexPrevious"), new_page);
        }))?;
    }

    let pk = page_kl(directory, new_page);
    let mut new_node = STLedgerEntry::new(pk);
    new_node.set_field_h256(sf("sfRootIndex"), directory.key);
    new_node.set_field_v256(sf("sfIndexes"), vec_to_v256(vec![key]));
    if new_page != 1 {
        new_node.set_field_u64(sf("sfIndexPrevious"), new_page - 1);
    }
    describe(&mut new_node);
    view.insert(Arc::new(new_node))?;

    Ok(Some(new_page))
}

// ---------------------------------------------------------------------------
// Public directory API
// ---------------------------------------------------------------------------

pub fn dir_append(
    view: &mut dyn ApplyView,
    directory: &Keylet,
    key: Uint256,
    describe: &dyn Fn(&mut STObject),
) -> Result<Option<u64>, ViewError> {
    dir_add(view, true, directory, key, describe)
}

pub fn dir_insert(
    view: &mut dyn ApplyView,
    directory: &Keylet,
    key: Uint256,
    describe: &dyn Fn(&mut STObject),
) -> Result<Option<u64>, ViewError> {
    dir_add(view, false, directory, key, describe)
}

pub fn dir_add(
    view: &mut dyn ApplyView,
    preserve_order: bool,
    directory: &Keylet,
    key: Uint256,
    describe: &dyn Fn(&mut STObject),
) -> Result<Option<u64>, ViewError> {
    let Some(root) = view.peek(*directory)? else {
        let page = create_root(view, directory, key, describe)?;
        return Ok(Some(page));
    };

    let (page, node, mut indexes) = find_previous_page(view, directory, &root)?;

    if indexes.len() < DIR_NODE_MAX_ENTRIES {
        let page = insert_key(view, &node, page, preserve_order, &mut indexes, key)?;
        return Ok(Some(page));
    }

    insert_page(view, page, &node, 0, &root, key, directory, describe)
}

pub fn dir_remove(
    view: &mut dyn ApplyView,
    directory: &Keylet,
    page: u64,
    key: Uint256,
    keep_root: bool,
) -> Result<bool, ViewError> {
    let page_keylet = page_kl(directory, page);
    let Some(node) = view.peek(page_keylet)? else {
        return Ok(false);
    };

    let root_page: u64 = 0;
    let mut entries = v256_to_vec(&node.get_field_v256(sf("sfIndexes")));
    let Some(pos) = entries.iter().position(|k| *k == key) else {
        return Ok(false);
    };
    entries.remove(pos);

    let node = sle_update(&node, |obj| {
        obj.set_field_v256(sf("sfIndexes"), vec_to_v256(entries.clone()));
    });
    view.update(Arc::clone(&node))?;

    if !entries.is_empty() {
        return Ok(true);
    }

    let mut prev_page = node.get_field_u64(sf("sfIndexPrevious"));
    let mut next_page = node.get_field_u64(sf("sfIndexNext"));

    if page == root_page {
        if next_page == page && prev_page != page {
            return Err(ViewError::Conversion(
                "Directory chain: fwd link broken".into(),
            ));
        }
        if prev_page == page && next_page != page {
            return Err(ViewError::Conversion(
                "Directory chain: rev link broken".into(),
            ));
        }

        if next_page == prev_page && next_page != page {
            let last = view
                .peek(page_kl(directory, next_page))?
                .ok_or_else(|| ViewError::Conversion("Directory chain: fwd link broken.".into()))?;
            let last_idx = v256_to_vec(&last.get_field_v256(sf("sfIndexes")));
            if last_idx.is_empty() {
                let root = sle_update(&node, |obj| {
                    obj.set_field_u64(sf("sfIndexNext"), page);
                    obj.set_field_u64(sf("sfIndexPrevious"), page);
                });
                view.update(root)?;
                view.erase(last)?;
                next_page = page;
                prev_page = page;
            }
        }

        if keep_root {
            return Ok(true);
        }

        if next_page == page && prev_page == page {
            view.erase(node)?;
        }

        return Ok(true);
    }

    // Non-root page
    if next_page == page || prev_page == page {
        return Err(ViewError::Conversion("Directory chain: link broken".into()));
    }

    let prev = view
        .peek(page_kl(directory, prev_page))?
        .ok_or_else(|| ViewError::Conversion("Directory chain: fwd link broken".into()))?;
    let prev = sle_update(&prev, |obj| {
        obj.set_field_u64(sf("sfIndexNext"), next_page);
    });
    view.update(Arc::clone(&prev))?;

    let next = view
        .peek(page_kl(directory, next_page))?
        .ok_or_else(|| ViewError::Conversion("Directory chain: rev link broken".into()))?;
    let next = sle_update(&next, |obj| {
        obj.set_field_u64(sf("sfIndexPrevious"), prev_page);
    });
    view.update(Arc::clone(&next))?;

    view.erase(node)?;

    if next_page != root_page
        && next.get_field_u64(sf("sfIndexNext")) == root_page
        && v256_to_vec(&next.get_field_v256(sf("sfIndexes"))).is_empty()
    {
        view.erase(next)?;

        let prev = view
            .peek(page_kl(directory, prev_page))?
            .ok_or_else(|| ViewError::Conversion("prev disappeared".into()))?;
        let prev = sle_update(&prev, |obj| {
            obj.set_field_u64(sf("sfIndexNext"), root_page);
        });
        view.update(prev)?;

        let root = view
            .peek(page_kl(directory, root_page))?
            .ok_or_else(|| ViewError::Conversion("Directory chain: root link broken.".into()))?;
        view.update(sle_update(&root, |obj| {
            obj.set_field_u64(sf("sfIndexPrevious"), prev_page);
        }))?;

        next_page = root_page;
    }

    if !keep_root && next_page == root_page && prev_page == root_page {
        let prev = view
            .peek(page_kl(directory, prev_page))?
            .ok_or_else(|| ViewError::Conversion("Directory chain: fwd link broken.".into()))?;
        if v256_to_vec(&prev.get_field_v256(sf("sfIndexes"))).is_empty() {
            view.erase(prev)?;
        }
    }

    Ok(true)
}

/// Delete an empty directory root, including the legacy empty terminal-page
/// cleanup performed by rippled's `ApplyView::emptyDirDelete`.
pub fn empty_dir_delete(view: &mut dyn ApplyView, directory: &Keylet) -> Result<bool, ViewError> {
    let Some(root) = view.peek(*directory)? else {
        return Ok(false);
    };
    if directory.entry_type != protocol::LedgerEntryType::DirectoryNode
        || root.get_field_h256(sf("sfRootIndex")) != directory.key
    {
        return Err(ViewError::Conversion(
            "emptyDirDelete: invalid directory root".into(),
        ));
    }
    if !v256_to_vec(&root.get_field_v256(sf("sfIndexes"))).is_empty() {
        return Ok(false);
    }

    let mut prev_page = root.get_field_u64(sf("sfIndexPrevious"));
    let mut next_page = root.get_field_u64(sf("sfIndexNext"));
    if next_page == 0 && prev_page != 0 {
        return Err(ViewError::Conversion(
            "Directory chain: fwd link broken".into(),
        ));
    }
    if prev_page == 0 && next_page != 0 {
        return Err(ViewError::Conversion(
            "Directory chain: rev link broken".into(),
        ));
    }

    let mut root = root;
    if next_page == prev_page && next_page != 0 {
        let last = view
            .peek(page_kl(directory, next_page))?
            .ok_or_else(|| ViewError::Conversion("Directory chain: fwd link broken.".into()))?;
        if !v256_to_vec(&last.get_field_v256(sf("sfIndexes"))).is_empty() {
            return Ok(false);
        }
        root = sle_update(&root, |obj| {
            obj.set_field_u64(sf("sfIndexNext"), 0);
            obj.set_field_u64(sf("sfIndexPrevious"), 0);
        });
        view.update(Arc::clone(&root))?;
        view.erase(last)?;
        next_page = 0;
        prev_page = 0;
    }

    if next_page == 0 && prev_page == 0 {
        view.erase(root)?;
    }
    Ok(true)
}

/// Delete every page of a directory in link order, invoking `callback` for
/// every contained key exactly like rippled's `ApplyView::dirDelete`.
pub fn dir_delete(
    view: &mut dyn ApplyView,
    directory: &Keylet,
    callback: &mut dyn FnMut(Uint256),
) -> Result<bool, ViewError> {
    let mut page = 0_u64;
    let mut visited = std::collections::BTreeSet::new();
    loop {
        if !visited.insert(page) {
            return Err(ViewError::Conversion(
                "Directory chain: cycle detected".into(),
            ));
        }
        let Some(node) = view.peek(page_kl(directory, page))? else {
            return Ok(false);
        };
        for key in v256_to_vec(&node.get_field_v256(sf("sfIndexes"))) {
            callback(key);
        }
        let next = if node.is_field_present(sf("sfIndexNext")) {
            Some(node.get_field_u64(sf("sfIndexNext")))
        } else {
            None
        };
        view.erase(node)?;
        let Some(next) = next else {
            return Ok(true);
        };
        page = next;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{Fees, LedgerHeader, RawView, ReadView, ReadViewTx};
    use protocol::{ApplyFlags, Rules, XRPAmount};

    #[derive(Debug)]
    struct FaultingDirectoryView {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
        fail: Option<Uint256>,
    }

    impl ReadView for FaultingDirectoryView {
        fn open(&self) -> bool {
            false
        }
        fn header(&self) -> LedgerHeader {
            LedgerHeader::default()
        }
        fn fees(&self) -> Fees {
            Fees::default()
        }
        fn rules(&self) -> Rules {
            Rules::default()
        }
        fn exists(&self, keylet: Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&keylet.key))
        }
        fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            if self.fail == Some(keylet.key) {
                return Err(ViewError::Conversion(
                    "injected directory SHAMap read failure".into(),
                ));
            }
            Ok(self.entries.get(&keylet.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.values().cloned().collect())
        }
        fn tx_exists(&self, _: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }

    impl RawView for FaultingDirectoryView {
        fn raw_erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.entries.remove(sle.key());
            Ok(())
        }
        fn raw_insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.entries.insert(*sle.key(), sle);
            Ok(())
        }
        fn raw_replace(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.entries.insert(*sle.key(), sle);
            Ok(())
        }
        fn raw_destroy_xrp(&mut self, _: XRPAmount) -> Result<(), ViewError> {
            Ok(())
        }
    }

    impl ApplyView for FaultingDirectoryView {
        fn flags(&self) -> ApplyFlags {
            ApplyFlags::NONE
        }
        fn peek(&mut self, keylet: Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            self.read(keylet)
        }
        fn insert(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.raw_insert(sle)
        }
        fn update(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.raw_replace(sle)
        }
        fn erase(&mut self, sle: Arc<STLedgerEntry>) -> Result<(), ViewError> {
            self.raw_erase(sle)
        }
        fn destroy_xrp(&mut self, fee: XRPAmount) -> Result<(), ViewError> {
            self.raw_destroy_xrp(fee)
        }
    }

    fn directory_page(
        directory: &Keylet,
        page: u64,
        indexes: Vec<Uint256>,
        previous: u64,
        next: u64,
    ) -> Arc<STLedgerEntry> {
        let mut sle = STLedgerEntry::new(page_kl(directory, page));
        sle.set_field_h256(sf("sfRootIndex"), directory.key);
        sle.set_field_v256(sf("sfIndexes"), vec_to_v256(indexes));
        if previous != 0 {
            sle.set_field_u64(sf("sfIndexPrevious"), previous);
        }
        if next != 0 {
            sle.set_field_u64(sf("sfIndexNext"), next);
        }
        Arc::new(sle)
    }

    fn faulting_view(
        entries: impl IntoIterator<Item = Arc<STLedgerEntry>>,
        fail: Keylet,
    ) -> FaultingDirectoryView {
        FaultingDirectoryView {
            entries: entries.into_iter().map(|sle| (*sle.key(), sle)).collect(),
            fail: Some(fail.key),
        }
    }

    #[test]
    fn owner_directory_description_sets_canonical_owner_field() {
        let account = AccountID::from_slice(&[0xA5; 20]).expect("account width");
        let mut directory = STObject::new(sf("sfGeneric"));

        describe_owner_dir(account)(&mut directory);

        assert_eq!(directory.get_account_id(sf("sfOwner")), account);
    }

    // reference: `ApplyView::dirAdd` -> `directory::insertPage` in
    // `xrpl/ledger/ApplyView.cpp`, guarded by
    // `!view.rules().enabled(fixDirectoryLimit) && page >= kDirNodeMaxPages`.

    #[test]
    fn page_limit_enforced_when_fix_directory_limit_disabled() {
        assert!(!directory_page_limit_exceeded(
            DIR_NODE_MAX_PAGES - 1,
            false
        ));
        assert!(directory_page_limit_exceeded(DIR_NODE_MAX_PAGES, false));
        assert!(directory_page_limit_exceeded(DIR_NODE_MAX_PAGES + 1, false));
    }

    #[test]
    fn page_limit_bypassed_when_fix_directory_limit_enabled() {
        assert!(!directory_page_limit_exceeded(DIR_NODE_MAX_PAGES, true));
        assert!(!directory_page_limit_exceeded(DIR_NODE_MAX_PAGES + 1, true));
        assert!(!directory_page_limit_exceeded(u64::MAX - 1, true));
    }

    #[test]
    fn directory_root_read_failure_is_not_treated_as_absence() {
        let directory = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_array([1; 20]));
        let mut view = faulting_view([], directory);
        assert!(dir_append(&mut view, &directory, Uint256::from_u64(1), &|_| {}).is_err());
        assert!(view.entries.is_empty());
    }

    #[test]
    fn append_terminal_page_read_failure_is_not_treated_as_a_new_page() {
        let directory = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_array([2; 20]));
        let root = directory_page(
            &directory,
            0,
            (0..DIR_NODE_MAX_ENTRIES)
                .map(|value| Uint256::from_u64(value as u64 + 1))
                .collect(),
            1,
            1,
        );
        let mut view = faulting_view([root], page_kl(&directory, 1));
        assert!(dir_append(&mut view, &directory, Uint256::from_u64(100), &|_| {}).is_err());
        assert!(!view.entries.contains_key(&page_kl(&directory, 2).key));
    }

    #[test]
    fn remove_page_and_link_read_failures_propagate_without_relinking() {
        let directory = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_array([3; 20]));
        let key = Uint256::from_u64(7);
        let root = directory_page(&directory, 0, Vec::new(), 2, 1);
        let page = directory_page(&directory, 1, vec![key], 0, 2);
        let next = directory_page(&directory, 2, vec![Uint256::from_u64(8)], 1, 0);

        let mut page_fault = faulting_view(
            [Arc::clone(&root), Arc::clone(&page), Arc::clone(&next)],
            page_kl(&directory, 1),
        );
        assert!(dir_remove(&mut page_fault, &directory, 1, key, true).is_err());

        let mut prev_fault = faulting_view(
            [Arc::clone(&root), Arc::clone(&page), Arc::clone(&next)],
            page_kl(&directory, 0),
        );
        assert!(dir_remove(&mut prev_fault, &directory, 1, key, true).is_err());

        let mut next_fault = faulting_view([root, page, next], page_kl(&directory, 2));
        assert!(dir_remove(&mut next_fault, &directory, 1, key, true).is_err());
    }

    #[test]
    fn empty_directory_legacy_last_page_read_failure_propagates() {
        let directory = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_array([4; 20]));
        let root = directory_page(&directory, 0, Vec::new(), 1, 1);
        let mut view = faulting_view([root], page_kl(&directory, 1));
        assert!(empty_dir_delete(&mut view, &directory).is_err());
        assert!(view.entries.contains_key(&directory.key));
    }

    #[test]
    fn removing_last_root_key_also_removes_a_legacy_empty_terminal_page() {
        let directory = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_array([6; 20]));
        let key = Uint256::from_u64(9);
        let root = directory_page(&directory, 0, vec![key], 1, 1);
        let last = directory_page(&directory, 1, Vec::new(), 0, 0);
        let mut view = FaultingDirectoryView {
            entries: [root, last]
                .into_iter()
                .map(|sle| (*sle.key(), sle))
                .collect(),
            fail: None,
        };
        assert_eq!(dir_remove(&mut view, &directory, 0, key, false), Ok(true));
        assert!(view.entries.is_empty());
    }

    #[test]
    fn directory_delete_rejects_an_explicit_link_cycle() {
        let directory = protocol::owner_dir_keylet(basics::base_uint::Uint160::from_array([5; 20]));
        let root = directory_page(&directory, 0, Vec::new(), 1, 1);
        let page = directory_page(&directory, 1, Vec::new(), 0, 1);
        let mut view = FaultingDirectoryView {
            entries: [root, page]
                .into_iter()
                .map(|sle| (*sle.key(), sle))
                .collect(),
            fail: None,
        };
        assert!(dir_delete(&mut view, &directory, &mut |_| {}).is_err());
    }
}
