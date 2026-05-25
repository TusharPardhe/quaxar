//! Directory helpers matching reference `xrpl::directory` namespace and
//! `ApplyView::dirAdd`/`dirRemove`/`dirAppend`/`dirInsert`/`dirDelete`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{Keylet, STLedgerEntry, STObject, STVector256};

use crate::read_view::ViewError;
use crate::views::apply_view::ApplyView;

pub const DIR_NODE_MAX_ENTRIES: usize = 32;

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
        let from_peek = view.peek(keylet)?;
        let from_read = if from_peek.is_none() {
            let r = view.read(keylet).ok().flatten();
            static DIR_READ_LOG: std::sync::atomic::AtomicU32 =
                std::sync::atomic::AtomicU32::new(0);
            if DIR_READ_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
                tracing::debug!(target: "ledger",
                    "[dir_debug] find_previous_page: dir={:?} page={} peek=None read={}",
                    &directory.key.data()[..4],
                    page,
                    if r.is_some() { "Some" } else { "None" }
                );
            }
            r
        } else {
            None
        };
        from_peek.or(from_read).ok_or_else(|| {
            static DIR_MISS_LOG: std::sync::atomic::AtomicU32 =
                std::sync::atomic::AtomicU32::new(0);
            if DIR_MISS_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
                tracing::warn!(target: "ledger",
                    "[dir_debug] BROKEN CHAIN: dir={:?} page={} key={:?}",
                    &directory.key.data()[..4],
                    page,
                    &keylet.key.data()[..4]
                );
            }
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

    view.raw_replace(sle_update(node, |obj| {
        obj.set_field_u64(sf("sfIndexNext"), new_page);
    }))?;

    view.raw_replace(sle_update(next, |obj| {
        obj.set_field_u64(sf("sfIndexPrevious"), new_page);
    }))?;

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
    // Use read fallback: peek checks sandbox cache; read goes through NuDB fetcher.
    let from_peek = view.peek(*directory)?;
    let root = if from_peek.is_none() {
        let r = view.read(*directory).ok().flatten();
        static DIR_ROOT_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if DIR_ROOT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
            tracing::debug!(target: "ledger",
                "[dir_debug] dir_add root: dir={:?} peek=None read={}",
                &directory.key.data()[..4],
                if r.is_some() {
                    "Some"
                } else {
                    "None (will create new root)"
                }
            );
        }
        r
    } else {
        from_peek
    };

    if root.is_none() {
        let page = create_root(view, directory, key, describe)?;
        return Ok(Some(page));
    }
    let root = root.unwrap();

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
    // Use read fallback so NuDB-backed pages are found even if not yet in sandbox cache.
    let Some(node) = view
        .peek(page_kl(directory, page))?
        .or_else(|| view.read(page_kl(directory, page)).ok().flatten())
    else {
        return Ok(false);
    };

    let root_page: u64 = 0;
    let mut entries = v256_to_vec(&node.get_field_v256(sf("sfIndexes")));
    let Some(pos) = entries.iter().position(|k| *k == key) else {
        return Ok(false);
    };
    entries.remove(pos);

    view.update(sle_update(&node, |obj| {
        obj.set_field_v256(sf("sfIndexes"), vec_to_v256(entries.clone()));
    }))?;

    if !entries.is_empty() {
        return Ok(true);
    }

    let prev_page = node.get_field_u64(sf("sfIndexPrevious"));
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

        if next_page == prev_page
            && next_page != page
            && let Some(last) = view
                .peek(page_kl(directory, next_page))?
                .or_else(|| view.read(page_kl(directory, next_page)).ok().flatten())
        {
            let last_idx = v256_to_vec(&last.get_field_v256(sf("sfIndexes")));
            if last_idx.is_empty() {
                let root_node = view
                    .peek(page_kl(directory, root_page))?
                    .or_else(|| view.read(page_kl(directory, root_page)).ok().flatten())
                    .ok_or_else(|| ViewError::Conversion("root disappeared".into()))?;
                view.update(sle_update(&root_node, |obj| {
                    obj.set_field_u64(sf("sfIndexNext"), page);
                    obj.set_field_u64(sf("sfIndexPrevious"), page);
                }))?;
                view.erase(last)?;
                next_page = page;
            }
        }

        if keep_root {
            return Ok(true);
        }

        if next_page == page && prev_page == page {
            let root_node = view
                .peek(page_kl(directory, root_page))?
                .or_else(|| view.read(page_kl(directory, root_page)).ok().flatten())
                .ok_or_else(|| ViewError::Conversion("root disappeared".into()))?;
            view.erase(root_node)?;
        }

        return Ok(true);
    }

    // Non-root page
    if next_page == page || prev_page == page {
        return Err(ViewError::Conversion("Directory chain: link broken".into()));
    }

    let prev = view
        .peek(page_kl(directory, prev_page))?
        .or_else(|| view.read(page_kl(directory, prev_page)).ok().flatten())
        .ok_or_else(|| ViewError::Conversion("Directory chain: fwd link broken".into()))?;
    view.update(sle_update(&prev, |obj| {
        obj.set_field_u64(sf("sfIndexNext"), next_page);
    }))?;

    let next = view
        .peek(page_kl(directory, next_page))?
        .or_else(|| view.read(page_kl(directory, next_page)).ok().flatten())
        .ok_or_else(|| ViewError::Conversion("Directory chain: rev link broken".into()))?;
    view.update(sle_update(&next, |obj| {
        obj.set_field_u64(sf("sfIndexPrevious"), prev_page);
    }))?;

    let node_to_erase = view
        .peek(page_kl(directory, page))?
        .or_else(|| view.read(page_kl(directory, page)).ok().flatten())
        .ok_or_else(|| ViewError::Conversion("page disappeared".into()))?;
    view.erase(node_to_erase)?;

    if let Some(next_ref) = view
        .peek(page_kl(directory, next_page))?
        .or_else(|| view.read(page_kl(directory, next_page)).ok().flatten())
        && next_page != root_page
        && next_ref.get_field_u64(sf("sfIndexNext")) == root_page
        && v256_to_vec(&next_ref.get_field_v256(sf("sfIndexes"))).is_empty()
    {
        view.erase(next_ref)?;

        let prev_ref = view
            .peek(page_kl(directory, prev_page))?
            .or_else(|| view.read(page_kl(directory, prev_page)).ok().flatten())
            .ok_or_else(|| ViewError::Conversion("prev disappeared".into()))?;
        view.update(sle_update(&prev_ref, |obj| {
            obj.set_field_u64(sf("sfIndexNext"), root_page);
        }))?;

        let root = view
            .peek(page_kl(directory, root_page))?
            .or_else(|| view.read(page_kl(directory, root_page)).ok().flatten())
            .ok_or_else(|| ViewError::Conversion("root disappeared".into()))?;
        view.update(sle_update(&root, |obj| {
            obj.set_field_u64(sf("sfIndexPrevious"), prev_page);
        }))?;

        next_page = root_page;
    }

    if !keep_root
        && next_page == root_page
        && prev_page == root_page
        && let Some(pf) = view
            .peek(page_kl(directory, prev_page))?
            .or_else(|| view.read(page_kl(directory, prev_page)).ok().flatten())
        && v256_to_vec(&pf.get_field_v256(sf("sfIndexes"))).is_empty()
    {
        view.erase(pf)?;
    }

    Ok(true)
}
