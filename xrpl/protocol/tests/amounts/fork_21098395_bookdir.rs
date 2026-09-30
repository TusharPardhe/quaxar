//! Canonical testnet fixture for ledger 21,098,395, OfferCreate
//! 4487A3573A4C64DB6A76BD171C52C0DA5074C38A2114A6DE2B6AD16EB3842224.
//!
//! The transaction and created Offer blobs below were returned by testnet's
//! validated `tx` and `ledger_entry` RPC methods. They are intentionally parsed
//! rather than re-created from JSON so this regression covers STAmount wire
//! deserialization as well as the historical BookDirectory bytes.
//!
//! Important: the stored historical directory quality (`651F…E761`) does not
//! equal the result specified by rippled HEAD's `getRate`: STAmount.cpp:422-443
//! calls `divide(offerIn, offerOut, noIssue())`; divide at :1227-1269 on the
//! two deserialized amounts returns `6521FD1CD85EC08E`. `OfferCreate.cpp:654-667`
//! initializes placement `uRate` with that same `getRate`, and :929-983 stores
//! `uRate` in both the DirectoryNode and Offer. The canonical historical SLE is
//! therefore retained as an observed ledger-data fixture, not used as a false
//! arithmetic oracle or special-cased by production amount math.

use basics::{base_uint::Uint256, string_utilities::str_unhex};
use protocol::{
    Book, LedgerEntryType, STLedgerEntry, STTx, SerialIter, StBase, get_book_base,
    get_field_by_symbol, get_rate,
};

const TX_BLOB: &str = "12000722000100002300001B59240140D8452A324C39AF201B0141EFAD644153E32073B3859065D4838D7EA4C68000424F4F4B000000000000000000000000000000007CAFC2EF1F9A6DF899DAD7FA8A7843F35414257268400000000000000C7321EDC3E77C9D18A42E888E940BCA54C886CED651C73670A0E3E2F6AFD84274BCD32274402DCFAEE0CC9AC7F101587916D7FFCEEE570EDA7480C3CB75703A09D3CA9FFA74BAACFA5CE8F00922B74AC58DBBFAF8E68EA52A8A84AF3C90B1A6D8C0B66F5B0181149A53DFA00A6E256730893D3BC50B47BC7CCB1C6C";
const OFFER_BLOB: &str = "11006F2200010000240140D845250141EF9B2A324C39AF330000000000000000340000000000000000554487A3573A4C64DB6A76BD171C52C0DA5074C38A2114A6DE2B6AD16EB384222450104B8BF7FAC704C646021053E4FC964CD5680B35CFDBE1BDA2651F8C1A7367E761644153E32073B3859065D4838D7EA4C68000424F4F4B000000000000000000000000000000007CAFC2EF1F9A6DF899DAD7FA8A7843F35414257281149A53DFA00A6E256730893D3BC50B47BC7CCB1C6C";
const OFFER_INDEX: &str = "B1367B647AB6C19E62F2FCCB5EB7D47EC83D157CC402CABDDE7C00C05D320A97";
const HISTORICAL_BOOK_DIRECTORY: &str =
    "4B8BF7FAC704C646021053E4FC964CD5680B35CFDBE1BDA2651F8C1A7367E761";
const HISTORICAL_DIRECTORY_QUALITY: u64 = 0x651F_8C1A_7367_E761;
const RIPPLED_HEAD_GET_RATE_QUALITY: u64 = 0x6521_FD1C_D85E_C08E;

fn decoded_tx() -> STTx {
    let bytes = str_unhex(TX_BLOB).expect("canonical transaction hex");
    STTx::from_serial_iter(&mut SerialIter::new(&bytes))
}

fn decoded_offer() -> STLedgerEntry {
    let bytes = str_unhex(OFFER_BLOB).expect("canonical offer hex");
    STLedgerEntry::from_serial_iter(
        &mut SerialIter::new(&bytes),
        Uint256::from_hex(OFFER_INDEX).expect("canonical offer index"),
    )
}

#[test]
fn fork_21098395_deserializes_the_real_offer_and_keeps_its_canonical_directory() {
    let tx = decoded_tx();
    let offer = decoded_offer();
    let historical = Uint256::from_hex(HISTORICAL_BOOK_DIRECTORY).expect("historical directory");

    assert_eq!(
        tx.get_transaction_id().to_string(),
        "4487A3573A4C64DB6A76BD171C52C0DA5074C38A2114A6DE2B6AD16EB3842224"
    );
    assert_eq!(offer.get_type(), LedgerEntryType::Offer);
    assert_eq!(
        offer.get_field_h256(get_field_by_symbol("sfBookDirectory")),
        historical
    );
    assert_eq!(
        offer
            .get_field_h256(get_field_by_symbol("sfBookDirectory"))
            .data()[24..32],
        HISTORICAL_DIRECTORY_QUALITY.to_be_bytes()
    );
    assert_eq!(
        offer
            .get_field_amount(get_field_by_symbol("sfTakerPays"))
            .xrp()
            .drops(),
        95_669_745_624_515_984
    );
    assert_eq!(
        offer
            .get_field_amount(get_field_by_symbol("sfTakerGets"))
            .text(),
        "1"
    );
}

#[test]
fn fork_21098395_raw_serialized_amounts_match_rippled_head_get_rate_not_the_historical_key() {
    let tx = decoded_tx();
    let offer = decoded_offer();
    let gets = tx.get_field_amount(get_field_by_symbol("sfTakerGets"));
    let pays = tx.get_field_amount(get_field_by_symbol("sfTakerPays"));

    // The real transaction and created Offer serialize byte-identical amounts.
    assert_eq!(
        gets,
        offer.get_field_amount(get_field_by_symbol("sfTakerGets"))
    );
    assert_eq!(
        pays,
        offer.get_field_amount(get_field_by_symbol("sfTakerPays"))
    );
    assert_eq!(get_rate(&gets, &pays), RIPPLED_HEAD_GET_RATE_QUALITY);
    assert_ne!(RIPPLED_HEAD_GET_RATE_QUALITY, HISTORICAL_DIRECTORY_QUALITY);

    let book = Book::new(pays.asset(), gets.asset(), None);
    let base = get_book_base(book);
    let mut reconstructed = *base.data();
    reconstructed[24..32].copy_from_slice(&RIPPLED_HEAD_GET_RATE_QUALITY.to_be_bytes());
    assert_ne!(
        Uint256::from_array(reconstructed),
        offer.get_field_h256(get_field_by_symbol("sfBookDirectory")),
        "rippled HEAD's documented getRate path cannot reconstruct this historical directory"
    );
}
