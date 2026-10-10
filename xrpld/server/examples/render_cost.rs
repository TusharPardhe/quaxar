//! Where does JSON rendering time go for a ledger entry?
//! cargo run --release -p server --example render_cost
use std::hint::black_box;
use std::time::Instant;

use protocol::{
    AccountID, Issue, JsonOptions, LedgerEntryType, STAmount, STLedgerEntry, SerialIter, StBase,
    currency_from_string, get_field_by_symbol as f, offer_keylet,
};

fn bench<T>(label: &str, iters: u32, mut op: impl FnMut() -> T) -> f64 {
    for _ in 0..iters / 10 {
        black_box(op());
    }
    let start = Instant::now();
    for _ in 0..iters {
        black_box(op());
    }
    let ns = start.elapsed().as_nanos() as f64 / f64::from(iters);
    println!("{label:<44} {ns:>9.0} ns");
    ns
}

fn main() {
    let owner = AccountID::from_array([7; 20]);
    let issuer = AccountID::from_array([9; 20]);
    let key = offer_keylet(
        basics::base_uint::Uint160::from_slice(owner.data()).unwrap(),
        42,
    )
    .key;
    let mut offer = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, key);
    offer.set_account_id(f("sfAccount"), owner);
    offer.set_field_u32(f("sfSequence"), 42);
    offer.set_field_u32(f("sfFlags"), 0);
    offer.set_field_amount(
        f("sfTakerPays"),
        STAmount::new_with_asset(
            f("sfTakerPays"),
            Issue::new(currency_from_string("USD"), issuer),
            3_076_560_200_591_725,
            -15,
            false,
        ),
    );
    offer.set_field_amount(f("sfTakerGets"), STAmount::new_native(6_000_000, false));
    offer.set_field_h256(f("sfBookDirectory"), key);
    offer.set_field_u64(f("sfBookNode"), 0);
    offer.set_field_u64(f("sfOwnerNode"), 0);
    offer.set_field_h256(f("sfPreviousTxnID"), key);
    offer.set_field_u32(f("sfPreviousTxnLgrSeq"), 21_305_409);
    let bytes = offer.get_serializer().data().to_vec();
    let json = offer.json(JsonOptions::NONE);
    println!(
        "Offer SLE: {} binary bytes, {} JSON bytes\n",
        bytes.len(),
        sonic_rs::to_string(&json).unwrap().len()
    );

    let n = 200_000;
    if std::env::args().nth(1).as_deref() == Some("tree") {
        let start = Instant::now();
        while start.elapsed().as_secs() < 8 {
            black_box(offer.json(JsonOptions::NONE));
        }
        return;
    }
    let decode = bench("1 decode bytes -> STLedgerEntry", n, || {
        STLedgerEntry::from_serial_iter(&mut SerialIter::new(&bytes), key)
    });
    let to_tree = bench("2 STLedgerEntry -> JsonValue tree", n, || {
        offer.json(JsonOptions::NONE)
    });
    let ser = bench("3 serialize tree -> JSON bytes", n, || {
        sonic_rs::to_vec(&json).unwrap()
    });
    let drop_tree = bench("4 clone+drop tree (alloc/free proxy)", n, || json.clone());
    let b58 = bench("  of which: one AccountID -> base58", n, || {
        protocol::to_base58(owner)
    });
    let hex = bench("  of which: one 32-byte hash -> hex", n, || key.to_string());
    let payload: Vec<u8> = (0..25_u8)
        .map(|i| i.wrapping_mul(97).wrapping_add(3))
        .collect();
    let alphabet = bs58::Alphabet::new(protocol::b58_fast::ALPHABET).unwrap();
    bench("  base58 encode only, bs58 crate (25 B)", n, || {
        bs58::encode(&payload)
            .with_alphabet(&alphabet)
            .into_string()
    });
    bench("  base58 encode only, b58_fast (25 B)", n, || {
        protocol::b58_fast::encode(&payload)
    });
    bench("  double SHA-256 checksum (21 B)", n, || {
        use sha2::{Digest, Sha256};
        Sha256::digest(Sha256::digest(&payload[..21]))
    });
    let total = decode + to_tree + ser;
    let iou = offer.get_field_amount(f("sfTakerPays"));
    let xrp = offer.get_field_amount(f("sfTakerGets"));
    let iou_ns = bench("  of which: IOU STAmount -> json", n, || {
        iou.json(JsonOptions::NONE)
    });
    let xrp_ns = bench("  of which: XRP STAmount -> json", n, || {
        xrp.json(JsonOptions::NONE)
    });
    println!("  amounts share ~{:.0}%", 100.0 * (iou_ns + xrp_ns) / total);
    println!(
        "\nper-entry pipeline (1+2+3): {total:.0} ns; tree build+serialize share {:.0}%; base58 x2 accounts ~{:.0}% ; hex x3 ~{:.0}%; drop proxy {drop_tree:.0} ns",
        100.0 * (to_tree + ser) / total,
        100.0 * 2.0 * b58 / total,
        100.0 * 3.0 * hex / total
    );
}
