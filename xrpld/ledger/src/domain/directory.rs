//! Read-only directory helper parity on top of the current typed `Ledger` owner.

use basics::base_uint::{Uint160, Uint256};
use protocol::{
    AccountID, Keylet, LedgerEntryType, STLedgerEntry, child_keylet, get_field_by_symbol,
    owner_dir_keylet, page_keylet,
};
use shamap::traversal::TraversalError;

use crate::read_view::{ReadView, ViewError};

impl From<ViewError> for TraversalError {
    fn from(value: ViewError) -> Self {
        match value {
            ViewError::Traversal(te) => te,
            _ => TraversalError::View,
        }
    }
}

fn sf_indexes() -> &'static protocol::SField {
    get_field_by_symbol("sfIndexes")
}

fn sf_index_next() -> &'static protocol::SField {
    get_field_by_symbol("sfIndexNext")
}

fn valid_directory_root(root: Keylet, assert_message: &str) -> bool {
    let valid = matches!(root.entry_type, LedgerEntryType::DirectoryNode);
    assert!(valid, "{assert_message}");
    valid
}

fn raw_account_id(id: AccountID) -> Uint160 {
    Uint160::from_slice(id.data()).expect("account width")
}

pub fn for_each_item<F>(view: &dyn ReadView, root: Keylet, mut f: F) -> Result<(), TraversalError>
where
    F: FnMut(Option<STLedgerEntry>),
{
    if !valid_directory_root(root, "xrpl::forEachItem : valid root type") {
        return Ok(());
    }

    let mut pos = root;
    loop {
        let Some(sle) = view.read(pos)? else {
            return Ok(());
        };

        for &key in sle.get_field_v256(sf_indexes()).value() {
            f(view.read(child_keylet(key))?.map(|arc| (*arc).clone()));
        }

        let next = sle.get_field_u64(sf_index_next());
        if next == 0 {
            return Ok(());
        }
        pos = page_keylet(root, next);
    }
}

pub fn for_each_item_after<F>(
    view: &dyn ReadView,
    root: Keylet,
    after: Uint256,
    hint: u64,
    mut limit: u32,
    mut f: F,
) -> Result<bool, TraversalError>
where
    F: FnMut(Option<STLedgerEntry>) -> bool,
{
    if !valid_directory_root(root, "xrpl::forEachItemAfter : valid root type") {
        return Ok(false);
    }

    let mut current_index = root;

    if after.is_non_zero() {
        let hint_index = page_keylet(root, hint);
        if let Some(hint_dir) = view.read(hint_index)? {
            for &key in hint_dir.get_field_v256(sf_indexes()).value() {
                if key == after {
                    current_index = hint_index;
                    break;
                }
            }
        }

        let mut found = false;
        loop {
            let Some(owner_dir) = view.read(current_index)? else {
                return Ok(found);
            };

            for &key in owner_dir.get_field_v256(sf_indexes()).value() {
                if !found {
                    if key == after {
                        found = true;
                    }
                } else if f(view.read(child_keylet(key))?.map(|arc| (*arc).clone())) {
                    if limit <= 1 {
                        return Ok(found);
                    }
                    limit -= 1;
                }
            }

            let next = owner_dir.get_field_u64(sf_index_next());
            if next == 0 {
                return Ok(found);
            }
            current_index = page_keylet(root, next);
        }
    }

    loop {
        let Some(owner_dir) = view.read(current_index)? else {
            return Ok(true);
        };

        for &key in owner_dir.get_field_v256(sf_indexes()).value() {
            if f(view.read(child_keylet(key))?.map(|arc| (*arc).clone())) {
                if limit <= 1 {
                    return Ok(true);
                }
                limit -= 1;
            }
        }

        let next = owner_dir.get_field_u64(sf_index_next());
        if next == 0 {
            return Ok(true);
        }
        current_index = page_keylet(root, next);
    }
}

pub fn for_each_owner_item<F>(
    view: &dyn ReadView,
    id: AccountID,
    f: F,
) -> Result<(), TraversalError>
where
    F: FnMut(Option<STLedgerEntry>),
{
    for_each_item(view, owner_dir_keylet(raw_account_id(id)), f)
}

pub fn for_each_owner_item_after<F>(
    view: &dyn ReadView,
    id: AccountID,
    after: Uint256,
    hint: u64,
    limit: u32,
    f: F,
) -> Result<bool, TraversalError>
where
    F: FnMut(Option<STLedgerEntry>) -> bool,
{
    for_each_item_after(
        view,
        owner_dir_keylet(raw_account_id(id)),
        after,
        hint,
        limit,
        f,
    )
}

pub fn dir_is_empty(view: &dyn ReadView, key: Keylet) -> Result<bool, TraversalError> {
    let Some(sle_node) = view.read(key)? else {
        return Ok(true);
    };
    if !sle_node.get_field_v256(sf_indexes()).value().is_empty() {
        return Ok(false);
    }

    // The anchor page may be empty even when later pages still carry entries.
    Ok(sle_node.get_field_u64(sf_index_next()) == 0)
}

#[derive(Debug)]
pub struct Dir<'a> {
    view: &'a dyn ReadView,
    root: Keylet,
    sle: Option<STLedgerEntry>,
    indexes: Option<Vec<Uint256>>,
}

impl<'a> Dir<'a> {
    pub fn new(view: &'a dyn ReadView, key: Keylet) -> Result<Self, TraversalError> {
        let sle = view.read(key)?;
        let indexes = sle
            .as_ref()
            .map(|entry| entry.get_field_v256(sf_indexes()).value().to_vec());

        Ok(Self {
            view,
            root: key,
            sle: sle.map(|arc| (*arc).clone()),
            indexes,
        })
    }

    pub fn begin(&self) -> DirIter<'a> {
        let mut it = DirIter::new(self.view, self.root, self.root);
        if let Some(sle) = self.sle.clone() {
            it.sle = Some(sle);
            if let Some(indexes) = self.indexes.clone()
                && let Some(&first) = indexes.first()
            {
                it.indexes = Some(indexes);
                it.position = 0;
                it.index = first;
            }
        }
        it
    }

    pub fn end(&self) -> DirIter<'a> {
        DirIter::new(self.view, self.root, self.root)
    }
}

#[derive(Debug)]
pub struct DirIter<'a> {
    view: &'a dyn ReadView,
    root: Keylet,
    page: Keylet,
    index: Uint256,
    cache: Option<Option<STLedgerEntry>>,
    sle: Option<STLedgerEntry>,
    indexes: Option<Vec<Uint256>>,
    position: usize,
}

impl<'a> DirIter<'a> {
    fn new(view: &'a dyn ReadView, root: Keylet, page: Keylet) -> Self {
        Self {
            view,
            root,
            page,
            index: Uint256::zero(),
            cache: None,
            sle: None,
            indexes: None,
            position: 0,
        }
    }

    pub fn is_end(&self) -> bool {
        self.index.is_zero()
    }

    pub fn current(&mut self) -> Result<Option<&STLedgerEntry>, TraversalError> {
        assert!(
            self.index.is_non_zero(),
            "xrpl::const_iterator::operator* : nonzero index"
        );

        if self.cache.is_none() {
            self.cache = Some(view_read_owned(self.view, child_keylet(self.index))?);
        }

        Ok(self.cache.as_ref().and_then(|entry| entry.as_ref()))
    }

    pub fn advance(&mut self) -> Result<(), TraversalError> {
        assert!(
            self.index.is_non_zero(),
            "xrpl::const_iterator::operator++ : nonzero index"
        );

        let next_position = self.position + 1;
        if let Some(indexes) = self.indexes.as_ref()
            && next_position < indexes.len()
        {
            self.position = next_position;
            self.index = indexes[next_position];
            self.cache = None;
            return Ok(());
        }

        self.next_page()
    }

    pub fn next_page(&mut self) -> Result<(), TraversalError> {
        let next = self
            .sle
            .as_ref()
            .expect("xrpl::const_iterator::next_page : non-null SLE")
            .get_field_u64(sf_index_next());

        if next == 0 {
            self.page = self.root;
            self.index = Uint256::zero();
        } else {
            self.page = page_keylet(self.root, next);
            self.sle = view_read_owned(self.view, self.page)?;
            assert!(
                self.sle.is_some(),
                "xrpl::const_iterator::next_page : non-null SLE"
            );

            let indexes = self
                .sle
                .as_ref()
                .map(|entry| entry.get_field_v256(sf_indexes()).value().to_vec())
                .unwrap_or_default();

            self.indexes = Some(indexes.clone());
            if indexes.is_empty() {
                self.index = Uint256::zero();
            } else {
                self.position = 0;
                self.index = indexes[0];
            }
        }

        self.cache = None;
        Ok(())
    }

    pub fn page_size(&self) -> usize {
        self.indexes.as_ref().map_or(0, Vec::len)
    }

    pub fn page(&self) -> Keylet {
        self.page
    }

    pub fn index(&self) -> Uint256 {
        self.index
    }
}

impl PartialEq for DirIter<'_> {
    fn eq(&self, other: &Self) -> bool {
        debug_assert!(
            self.root.key == other.root.key,
            "xrpl::const_iterator::operator== : roots are matching"
        );
        self.page.key == other.page.key && self.index == other.index
    }
}

impl Eq for DirIter<'_> {}

fn view_read_owned(
    view: &dyn ReadView,
    k: Keylet,
) -> Result<Option<STLedgerEntry>, TraversalError> {
    Ok(view.read(k)?.map(|arc| (*arc).clone()))
}
