use basics::base_uint::Uint256;
use basics::string_utilities::sql_blob_literal;
use protocol::{
    AccountID, IOUAmount, Issue, JsonValue, LedgerEntryType, MPTAmount, MPTIssue, STAmount,
    STLedgerEntry, STObject, Serializer, StBase, TxMeta, currency_from_string, get_field_by_symbol,
    make_mpt_id,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn offer_payload(field: &'static protocol::SField, issuer: AccountID) -> STObject {
    let usd = currency_from_string("USD");
    let eur = currency_from_string("EUR");

    let mut payload = STObject::new(field);
    payload.set_account_id(get_field_by_symbol("sfAccount"), account(0x11));
    payload.set_field_amount(
        get_field_by_symbol("sfLowLimit"),
        STAmount::from_iou_amount(
            get_field_by_symbol("sfLowLimit"),
            IOUAmount::from_parts(10, 0).expect("IOU amount should normalize"),
            Issue::new(usd, issuer),
        ),
    );
    payload.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        STAmount::from_iou_amount(
            get_field_by_symbol("sfTakerGets"),
            IOUAmount::from_parts(20, 0).expect("IOU amount should normalize"),
            Issue::new(eur, account(0x44)),
        ),
    );
    payload.set_field_h192(
        get_field_by_symbol("sfMPTokenIssuanceID"),
        make_mpt_id(9, account(0x55)),
    );
    payload
}

#[test]
fn fix_mpt_delivered_amount_round_trips_canonical_metadata() {
    let mpt_issue = MPTIssue::new(make_mpt_id(7, account(0x71)));
    let delivered_amount = STAmount::from_mpt_amount(
        get_field_by_symbol("sfDeliveredAmount"),
        MPTAmount::from_value(800),
        mpt_issue,
    );
    let mut meta = TxMeta::new(hash(0x72), 73);
    meta.set_delivered_amount(Some(delivered_amount.clone()));

    let mut serializer = Serializer::default();
    meta.add_raw(&mut serializer, protocol::Ter::TES_SUCCESS, 3);
    let reparsed = TxMeta::from_raw(hash(0x72), 73, serializer.data());

    assert_eq!(reparsed.get_delivered_amount(), Some(&delivered_amount));
    assert!(
        reparsed
            .get_as_object()
            .is_field_present(get_field_by_symbol("sfDeliveredAmount"))
    );
}

#[test]
fn tx_meta_from_object_extracts_core_fields() {
    let mut affected_nodes = protocol::STArray::new(get_field_by_symbol("sfAffectedNodes"));
    let mut node = STObject::new(get_field_by_symbol("sfModifiedNode"));
    node.set_field_h256(get_field_by_symbol("sfLedgerIndex"), hash(0x01));
    node.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::Offer.code(),
    );
    node.set_field_object(
        get_field_by_symbol("sfFinalFields"),
        offer_payload(get_field_by_symbol("sfFinalFields"), account(0x33)),
    );
    affected_nodes.push_back(node);

    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 7);
    object.set_field_amount(
        get_field_by_symbol("sfDeliveredAmount"),
        STAmount::new_native(25, false),
    );
    object.set_field_h256(get_field_by_symbol("sfParentBatchID"), hash(0xAB));
    object.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);

    let meta = TxMeta::from_stobject(hash(0xCD), 44, object.clone());

    assert_eq!(meta.get_tx_id(), hash(0xCD));
    assert_eq!(meta.get_lgr_seq(), 44);
    assert_eq!(meta.get_result(), 0);
    assert_eq!(meta.get_index(), 7);
    assert_eq!(
        meta.get_delivered_amount(),
        Some(&STAmount::new_native(25, false))
    );
    assert_eq!(meta.get_parent_batch_id(), Some(hash(0xAB)));
    assert_eq!(meta.get_nodes().len(), 1);

    let canonical = meta.get_as_object();
    assert_eq!(
        canonical.get_field_u8(get_field_by_symbol("sfTransactionResult")),
        0
    );
    assert_eq!(
        canonical.get_field_u32(get_field_by_symbol("sfTransactionIndex")),
        7
    );
    assert_eq!(
        canonical.get_field_h256(get_field_by_symbol("sfParentBatchID")),
        hash(0xAB)
    );
    assert_eq!(
        canonical.get_field_amount(get_field_by_symbol("sfDeliveredAmount")),
        STAmount::new_native(25, false)
    );
    assert_eq!(
        canonical
            .get_field_array(get_field_by_symbol("sfAffectedNodes"))
            .len(),
        1
    );
}

#[test]
fn tx_meta_collects_affected_accounts_from_accounts_issues_and_mpt() {
    let issuer = account(0x33);
    let mut created = STObject::new(get_field_by_symbol("sfCreatedNode"));
    created.set_field_h256(get_field_by_symbol("sfLedgerIndex"), hash(0x01));
    created.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::Offer.code(),
    );
    created.set_field_object(
        get_field_by_symbol("sfNewFields"),
        offer_payload(get_field_by_symbol("sfNewFields"), issuer),
    );

    let mut affected_nodes = protocol::STArray::new(get_field_by_symbol("sfAffectedNodes"));
    affected_nodes.push_back(created);

    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 1);
    object.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);

    let meta = TxMeta::from_stobject(hash(0xFE), 88, object);
    let expected = std::collections::BTreeSet::from([
        account(0x11),
        issuer,
        account(0x44),
        MPTIssue::new(make_mpt_id(9, account(0x55))).issuer(),
    ]);
    assert_eq!(meta.get_affected_accounts(), expected);
}

#[test]
fn tx_meta_add_raw_sorts_nodes_and_round_trips() {
    let mut meta = TxMeta::new(hash(0x99), 90);
    meta.set_affected_node(hash(0xF0), get_field_by_symbol("sfModifiedNode"), 111);
    meta.set_affected_node(hash(0x0F), get_field_by_symbol("sfDeletedNode"), 222);

    let mut serializer = Serializer::default();
    meta.add_raw(&mut serializer, protocol::Ter::TES_SUCCESS, 12);

    let reparsed = TxMeta::from_raw(hash(0x99), 90, serializer.data());
    let indexes: Vec<_> = reparsed
        .get_nodes()
        .iter()
        .map(|node| node.get_field_h256(get_field_by_symbol("sfLedgerIndex")))
        .collect();

    assert_eq!(reparsed.get_result(), 0);
    assert_eq!(reparsed.get_index(), 12);
    assert_eq!(indexes, vec![hash(0x0F), hash(0xF0)]);
}

#[test]
fn tx_meta_get_affected_node_for_sle_creates_and_reuses_entry() {
    let mut meta = TxMeta::new(hash(0x77), 9);
    let sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, hash(0x42));

    let node = meta.get_affected_node_for_sle(&sle, get_field_by_symbol("sfCreatedNode"));
    node.set_field_object(
        get_field_by_symbol("sfNewFields"),
        STObject::new(get_field_by_symbol("sfNewFields")),
    );

    let same = meta.get_affected_node(hash(0x42));
    assert_eq!(same.fname(), get_field_by_symbol("sfCreatedNode"));
    assert_eq!(
        same.get_field_u16(get_field_by_symbol("sfLedgerEntryType")),
        LedgerEntryType::Offer.code()
    );
}

#[test]
fn tx_meta_json_and_raw_blob_follow_current_cpp_shapes() {
    let mut meta = TxMeta::new(hash(0x10), 2);
    meta.set_affected_node(hash(0x20), get_field_by_symbol("sfModifiedNode"), 3);

    let mut serializer = Serializer::default();
    meta.add_raw(&mut serializer, protocol::Ter::TES_SUCCESS, 4);
    let json = meta.get_json(protocol::JsonOptions::NONE);

    let JsonValue::Object(object) = json else {
        panic!("metadata json should be an object");
    };

    assert_eq!(
        object.get("TransactionResult"),
        Some(&JsonValue::String("tesSUCCESS".to_string()))
    );
    assert_eq!(
        object.get("TransactionIndex"),
        Some(&JsonValue::Unsigned(4))
    );
    assert!(sql_blob_literal(&serializer.data().to_vec()).starts_with("X'"));
}

#[test]
fn tx_meta_add_raw_rejects_invalid_ter_range() {
    let mut meta = TxMeta::new(hash(0x10), 2);
    let mut serializer = Serializer::default();

    let result = catch_unwind(AssertUnwindSafe(|| {
        meta.add_raw(&mut serializer, protocol::Ter::TEL_LOCAL_ERROR, 4);
    }));

    assert!(result.is_err());
}
