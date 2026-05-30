use super::common::*;
use basics::base_uint::Uint256;
use protocol::{LedgerEntryType, STLedgerEntry};
use std::collections::BTreeSet;

pub(super) fn is_root_book_directory(sle: &STLedgerEntry) -> bool {
    [
        "sfExchangeRate",
        "sfTakerPaysCurrency",
        "sfTakerPaysIssuer",
        "sfTakerPaysMPT",
        "sfTakerGetsCurrency",
        "sfTakerGetsIssuer",
        "sfTakerGetsMPT",
        "sfDomainID",
    ]
    .iter()
    .any(|field| sle.is_field_present(sf(field)))
}

pub(super) fn bad_book_exchange_rate(sle: &STLedgerEntry) -> bool {
    is_root_book_directory(sle)
        && (!sle.is_field_present(sf("sfExchangeRate"))
            || sle.get_field_u64(sf("sfExchangeRate")) != protocol::quality_from_key(*sle.key()))
}

pub(super) fn maybe_record_directory_root(
    roots: &mut BTreeSet<Uint256>,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) -> bool {
    if is_delete || after.is_none_or(|sle| sle.get_type() != LedgerEntryType::DirectoryNode) {
        return true;
    }
    let after = after.expect("checked above");
    let root_index = after.get_field_h256(sf("sfRootIndex"));

    if before.is_some_and(|sle| sle.get_field_h256(sf("sfRootIndex")) == root_index) {
        return true;
    }

    if *after.key() == root_index {
        return !bad_book_exchange_rate(after);
    }

    roots.insert(root_index);
    true
}
