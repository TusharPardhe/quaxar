//! rippled `RPCHelpers.cpp` `isRelatedToAccount` (#8274): the marker validator
//! for `account_lines`, `account_offers` and `account_channels` pagination.

use protocol::{STLedgerEntry, get_field_by_symbol};

/// UINT64 `sf*Node` fields that record a page number in an owner directory.
/// `sfBookNode` and `sfNFTokenOfferNode` are book-directory hints and are
/// deliberately absent; the coverage test below enforces the split.
pub const OWNER_DIR_NODE_FIELDS: [&str; 9] = [
    "sfOwnerNode",
    "sfLowNode",
    "sfHighNode",
    "sfDestinationNode",
    "sfIssuerNode",
    "sfSubjectNode",
    "sfSponseeNode",
    "sfLoanBrokerNode",
    "sfVaultNode",
];

/// True iff `sle`'s key is listed on one of the owner-directory pages of
/// the account named by its page-hint fields. `read_owner_dir_page(n)` reads
/// page `n` of that account's owner directory (rippled
/// `keylet::page(ownerDir, n)`); at most one read per present hint field.
pub fn is_related_to_account(
    sle: &STLedgerEntry,
    read_owner_dir_page: impl Fn(u64) -> Option<STLedgerEntry>,
) -> bool {
    let indexes = get_field_by_symbol("sfIndexes");
    OWNER_DIR_NODE_FIELDS.iter().any(|symbol| {
        let field = get_field_by_symbol(symbol);
        sle.is_field_present(field)
            && read_owner_dir_page(sle.get_field_u64(field))
                .is_some_and(|page| page.get_field_v256(indexes).value().contains(sle.key()))
    })
}

#[cfg(test)]
mod tests {
    use super::OWNER_DIR_NODE_FIELDS;
    use protocol::{SerializedTypeId, all_sfields};

    /// Mirrors rippled `RPCHelpers_test::testOwnerDirNodeFieldsCoverage`:
    /// every UINT64 `sf*Node` field is either an owner-directory page hint
    /// or one of the two book-directory hints.
    #[test]
    fn every_uint64_node_field_is_classified() {
        let book_dir = ["sfBookNode", "sfNFTokenOfferNode"];
        let mut seen = 0;
        for field in all_sfields() {
            let name = field.symbol_name();
            if field.field_type() != SerializedTypeId::UInt64 || !name.ends_with("Node") {
                continue;
            }
            seen += 1;
            assert!(
                OWNER_DIR_NODE_FIELDS.contains(&name) ^ book_dir.contains(&name),
                "{name} must be classified exactly once"
            );
        }
        assert_eq!(seen, OWNER_DIR_NODE_FIELDS.len() + book_dir.len());
    }
}
