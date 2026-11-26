//! Signed transaction blobs for the five payment types and the setup
//! transactions.

use crate::accounts::Key;
use protocol::{
    AMMCreateBuilder, AccountID, AccountSetBuilder, Currency, Issue, OfferCreateBuilder,
    PaymentBuilder, STAmount, STPath, STPathElement, STPathSet, TrustSetBuilder,
    get_field_by_symbol, path_asset::PathAsset, to_currency,
};

pub struct Signed {
    pub blob: String,
    #[allow(dead_code)] // kept for debugging submits
    pub hash: String,
}

/// Fields every transaction takes.
#[derive(Clone, Copy)]
pub struct Common {
    pub sequence: u32,
    pub fee_drops: u64,
    pub last_ledger: u32,
}

pub fn currency(code: &str) -> Currency {
    let mut c = Currency::default();
    assert!(to_currency(&mut c, code), "currency {code}");
    c
}

pub fn xrp(drops: u64) -> STAmount {
    STAmount::new_native(drops, false)
}

/// `mantissa × 10^exponent` units of `code` issued by `issuer`.
pub fn iou(code: Currency, issuer: AccountID, mantissa: u64, exponent: i32) -> STAmount {
    STAmount::new_with_asset(
        get_field_by_symbol("sfAmount"),
        Issue::new(code, issuer),
        mantissa,
        exponent,
        false,
    )
}

fn hex_upper(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 15)] as char);
    }
    out
}

fn finish(tx: &protocol::STTx) -> Signed {
    Signed {
        blob: hex_upper(tx.get_serializer().data()),
        hash: tx.get_transaction_id().to_string(),
    }
}

/// `account` signs with `signer` (its own key, or its RegularKey).
pub fn payment(
    account: AccountID,
    signer: &Key,
    destination: AccountID,
    amount: STAmount,
    send_max: Option<STAmount>,
    paths: Option<STPathSet>,
    c: Common,
) -> Signed {
    let mut b = PaymentBuilder::new(
        account,
        destination,
        amount,
        Some(c.sequence),
        Some(xrp(c.fee_drops)),
    )
    .set_last_ledger_sequence(c.last_ledger);
    if let Some(m) = send_max {
        b = b.set_send_max(m);
    }
    if let Some(p) = paths {
        b = b.set_paths(p);
    }
    let tx = b.build(&signer.public, &signer.secret).expect("sign payment");
    finish(tx.as_sttx())
}

pub fn trust_set(key: &Key, limit: STAmount, c: Common) -> Signed {
    let tx = TrustSetBuilder::new(key.account, Some(c.sequence), Some(xrp(c.fee_drops)))
        .set_limit_amount(limit)
        .set_last_ledger_sequence(c.last_ledger)
        .build(&key.public, &key.secret)
        .expect("sign trust set");
    finish(tx.as_sttx())
}

pub fn account_set_flag(key: &Key, flag: u32, c: Common) -> Signed {
    let tx = AccountSetBuilder::new(key.account, Some(c.sequence), Some(xrp(c.fee_drops)))
        .set_set_flag(flag)
        .set_last_ledger_sequence(c.last_ledger)
        .build(&key.public, &key.secret)
        .expect("sign account set");
    finish(tx.as_sttx())
}

pub fn amm_create(key: &Key, a: STAmount, b: STAmount, c: Common) -> Signed {
    let tx = AMMCreateBuilder::new(key.account, a, b, 0, Some(c.sequence), Some(xrp(c.fee_drops)))
        .set_last_ledger_sequence(c.last_ledger)
        .build(&key.public, &key.secret)
        .expect("sign amm create");
    finish(tx.as_sttx())
}

pub fn offer_create(key: &Key, taker_pays: STAmount, taker_gets: STAmount, c: Common) -> Signed {
    let tx = OfferCreateBuilder::new(
        key.account,
        taker_pays,
        taker_gets,
        Some(c.sequence),
        Some(xrp(c.fee_drops)),
    )
    .set_last_ledger_sequence(c.last_ledger)
    .build(&key.public, &key.secret)
    .expect("sign offer create");
    finish(tx.as_sttx())
}

/// One book step to `code` issued by `issuer`.
fn step(code: Currency, issuer: AccountID) -> STPathElement {
    STPathElement::from_optionals(None, Some(PathAsset::from(code)), Some(issuer))
}

/// Paths from XRP to currency index `dest`: `paths` alternatives of `steps`
/// book steps each, ending at `dest`. Each path visits distinct currencies,
/// and alternatives differ by their first currency.
pub fn path_set(issuer: AccountID, dest: usize, paths: usize, steps: usize) -> STPathSet {
    let n = crate::accounts::CURRENCIES.len();
    assert!(steps >= 1 && steps <= n && paths >= 1 && paths <= 6);
    let mut set = STPathSet::new(get_field_by_symbol("sfPaths"));
    for alt in 0..paths {
        // Currencies other than dest, rotated by the alternative.
        let others: Vec<usize> = (0..n).filter(|&i| i != dest).collect();
        let mut chosen: Vec<usize> = (0..steps - 1)
            .map(|k| others[(alt + k) % others.len()])
            .collect();
        chosen.push(dest);
        let elements = chosen
            .into_iter()
            .map(|i| step(currency(crate::accounts::CURRENCIES[i]), issuer))
            .collect();
        set.push_back(STPath::from_vec(elements));
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::derive;

    #[test]
    fn signed_payment_parses_and_verifies() {
        let a = derive("t", 1);
        let b = derive("t", 2);
        let c = Common { sequence: 7, fee_drops: 12, last_ledger: 100 };
        let s = payment(a.account, &a, b.account, xrp(1000), None, None, c);
        let bytes: Vec<u8> = (0..s.blob.len() / 2)
            .map(|i| u8::from_str_radix(&s.blob[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        let tx = protocol::STTx::try_from_serial_iter(&mut protocol::SerialIter::new(&bytes))
            .expect("parse");
        assert!(tx.check_sign(&protocol::Rules::default()).is_ok());
        assert_eq!(tx.get_transaction_id().to_string(), s.hash);
    }

    #[test]
    fn path_sets_have_requested_shape_and_distinct_currencies() {
        let issuer = derive("t", 0).account;
        for (paths, steps) in [(1, 1), (3, 3), (6, 8)] {
            let set = path_set(issuer, 3, paths, steps);
            assert_eq!(set.size(), paths);
            for path in set.iter() {
                assert_eq!(path.size(), steps);
                let mut seen = std::collections::HashSet::new();
                for e in path.iter() {
                    assert!(seen.insert(e.currency()), "repeated currency");
                }
            }
        }
    }
}
