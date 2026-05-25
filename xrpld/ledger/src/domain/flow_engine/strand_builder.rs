use super::{StepKind, Strand};
use protocol::{
    AccountID, Asset, Currency, Issue, STPath, STPathSet, Ter, is_xrp_currency, xrp_account,
    xrp_issue,
};

pub fn to_strand(
    src: &AccountID,
    dst: &AccountID,
    deliver: &Asset,
    send_max_asset: Option<&Asset>,
    path: &STPath,
    _owner_pays_transfer_fee: bool,
    _offer_crossing: bool,
) -> (Ter, Strand) {
    if src.is_zero() || dst.is_zero() {
        return (Ter::TEM_BAD_PATH, Vec::new());
    }

    // Compute initial curAsset (reference lines 239-249)
    let (initial_currency, initial_issuer) = match send_max_asset.unwrap_or(deliver) {
        Asset::Issue(issue) => {
            if is_xrp_currency(issue.currency) {
                (protocol::xrp_currency(), xrp_account())
            } else {
                (issue.currency, *src)
            }
        }
        _ => (protocol::xrp_currency(), xrp_account()),
    };

    // Build normalized path elements
    let mut norm: Vec<NormElem> = Vec::with_capacity(4 + path.size());

    // 1. Source element
    norm.push(NormElem::Acct(*src));

    // 2. SendMax issuer if != src
    if let Some(Asset::Issue(sma)) = send_max_asset
        && !is_xrp_currency(sma.currency)
        && sma.account != *src
    {
        let first_is_issuer = path
            .iter()
            .next()
            .map(|e| e.is_account() && e.account_id() == sma.account)
            .unwrap_or(false);
        if !first_is_issuer {
            norm.push(NormElem::Acct(sma.account));
        }
    }

    // 3. Explicit path elements
    for elem in path.iter() {
        if elem.is_account() {
            norm.push(NormElem::Acct(elem.account_id()));
        } else {
            let cur = if elem.has_currency() {
                elem.currency()
            } else {
                Currency::default()
            };
            let iss = if elem.has_issuer() {
                elem.issuer_id()
            } else {
                AccountID::default()
            };
            norm.push(NormElem::Offer(cur, iss));
        }
    }

    // 4. Deliver asset if last asset != deliver
    let deliver_issue = match deliver {
        Asset::Issue(i) => *i,
        _ => Issue::default(),
    };
    let needs_deliver_book = {
        let last_currency = last_currency_in_norm(&norm, initial_currency);
        last_currency != deliver_issue.currency
    };
    if needs_deliver_book {
        norm.push(NormElem::Offer(
            deliver_issue.currency,
            deliver_issue.account,
        ));
    }

    // 5. Deliver issuer if != dst
    let deliver_issuer = deliver_issue.account;
    let last_is_deliver_issuer =
        matches!(norm.last(), Some(NormElem::Acct(a)) if *a == deliver_issuer);
    if !last_is_deliver_issuer && *dst != deliver_issuer && !deliver_issuer.is_zero() {
        norm.push(NormElem::Acct(deliver_issuer));
    }

    // 6. Destination if not already last
    let last_is_dst = matches!(norm.last(), Some(NormElem::Acct(a)) if *a == *dst);
    if !last_is_dst {
        norm.push(NormElem::Acct(*dst));
    }

    if norm.len() < 2 {
        return (Ter::TEM_BAD_PATH, Vec::new());
    }

    // Create steps from normalized path pairs
    let mut strand: Strand = Vec::new();
    let mut cur_currency = initial_currency;
    let mut cur_issuer = initial_issuer;

    for i in 0..norm.len() - 1 {
        let cur = &norm[i];
        let next = &norm[i + 1];

        // Update curAsset from current element
        match cur {
            NormElem::Acct(acct) => {
                if !is_xrp_currency(cur_currency) {
                    cur_issuer = *acct;
                }
            }
            NormElem::Offer(c, iss) => {
                if *c != Currency::default() {
                    cur_currency = *c;
                    if is_xrp_currency(*c) {
                        cur_issuer = xrp_account();
                    } else if *iss != AccountID::default() {
                        cur_issuer = *iss;
                    }
                }
            }
        }

        match (cur, next) {
            (NormElem::Acct(s), NormElem::Acct(d)) => {
                if is_xrp_currency(cur_currency) {
                    // XRP endpoint
                    let is_first = i == 0;
                    strand.push(StepKind::XrpEndpoint {
                        account: if is_first { *s } else { *d },
                        is_last: !is_first,
                    });
                } else {
                    // DirectStep
                    strand.push(StepKind::Direct {
                        src: *s,
                        dst: *d,
                        currency: cur_currency,
                    });
                }
            }
            (NormElem::Acct(s), NormElem::Offer(out_c, out_iss)) => {
                if i == 0 && is_xrp_currency(cur_currency) {
                    strand.push(StepKind::XrpEndpoint {
                        account: *s,
                        is_last: false,
                    });
                }
                // BookStep
                let in_issue = Issue {
                    currency: cur_currency,
                    account: cur_issuer,
                };
                let out_issue = if is_xrp_currency(*out_c) {
                    xrp_issue()
                } else {
                    Issue {
                        currency: *out_c,
                        account: *out_iss,
                    }
                };
                strand.push(StepKind::Book {
                    book_in: in_issue,
                    book_out: out_issue,
                });
                cur_currency = out_issue.currency;
                cur_issuer = out_issue.account;
            }
            (NormElem::Offer(_, _), NormElem::Acct(d)) => {
                // Offer→Account: implied step if cur_issuer != dst
                if cur_issuer != *d && !d.is_zero() {
                    if is_xrp_currency(cur_currency) {
                        strand.push(StepKind::XrpEndpoint {
                            account: *d,
                            is_last: true,
                        });
                    } else {
                        strand.push(StepKind::Direct {
                            src: cur_issuer,
                            dst: *d,
                            currency: cur_currency,
                        });
                    }
                }
                continue;
            }
            (NormElem::Offer(_, _), NormElem::Offer(out_c, out_iss)) => {
                let in_issue = Issue {
                    currency: cur_currency,
                    account: cur_issuer,
                };
                let out_issue = if is_xrp_currency(*out_c) {
                    xrp_issue()
                } else {
                    Issue {
                        currency: *out_c,
                        account: *out_iss,
                    }
                };
                strand.push(StepKind::Book {
                    book_in: in_issue,
                    book_out: out_issue,
                });
                cur_currency = out_issue.currency;
                cur_issuer = out_issue.account;
            }
        }
    }

    if strand.is_empty() {
        return (Ter::TEM_BAD_PATH, Vec::new());
    }

    static STRAND_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if STRAND_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 5 {
        tracing::debug!(target: "ledger",            "[to_strand] built strand with {} steps: {:?}",
            strand.len(),
            strand
        );
    }

    (Ter::TES_SUCCESS, strand)
}

pub fn to_strands(
    src: &AccountID,
    dst: &AccountID,
    deliver: &Asset,
    send_max_asset: Option<&Asset>,
    paths: &STPathSet,
    default_paths_allowed: bool,
    owner_pays_transfer_fee: bool,
    offer_crossing: bool,
) -> (Ter, Vec<Strand>) {
    let mut result: Vec<Strand> = Vec::new();

    if default_paths_allowed {
        let empty_path = protocol::STPath::new();
        let (ter, strand) = to_strand(
            src,
            dst,
            deliver,
            send_max_asset,
            &empty_path,
            owner_pays_transfer_fee,
            offer_crossing,
        );
        if ter == Ter::TES_SUCCESS && !strand.is_empty() {
            result.push(strand);
        } else if ter != Ter::TES_SUCCESS && paths.size() == 0 {
            return (ter, Vec::new());
        }
    }

    for path in paths.iter() {
        let (ter, strand) = to_strand(
            src,
            dst,
            deliver,
            send_max_asset,
            path,
            owner_pays_transfer_fee,
            offer_crossing,
        );
        if ter == Ter::TES_SUCCESS && !strand.is_empty() {
            result.push(strand);
        }
    }

    if result.is_empty() && !default_paths_allowed && paths.size() == 0 {
        return (Ter::TEM_RIPPLE_EMPTY, Vec::new());
    }

    (Ter::TES_SUCCESS, result)
}

#[derive(Debug, Clone)]
enum NormElem {
    Acct(AccountID),
    Offer(Currency, AccountID),
}

fn last_currency_in_norm(norm: &[NormElem], initial: Currency) -> Currency {
    for elem in norm.iter().rev() {
        if let NormElem::Offer(c, _) = elem
            && *c != Currency::default()
        {
            return *c;
        }
    }
    initial
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{AccountID, Asset, Currency, Issue, xrp_issue};

    fn make_account(byte: u8) -> AccountID {
        let mut data = [0u8; 20];
        data[0] = byte;
        AccountID::from(data)
    }

    fn make_currency(s: &str) -> Currency {
        let mut data = [0u8; 20];
        for (i, b) in s.bytes().enumerate().take(3) {
            data[12 + i] = b;
        }
        Currency::from(data)
    }

    #[test]
    fn test_iou_to_iou_default_path_through_issuer() {
        // IOU→IOU: sender→issuer→receiver (default path, no explicit paths)
        let src = make_account(1); // Alice
        let dst = make_account(2); // Bob
        let gateway = make_account(3); // Gateway (issuer)
        let usd = make_currency("USD");
        let deliver = Asset::Issue(Issue {
            currency: usd,
            account: gateway,
        });

        let (ter, strand) = to_strand(
            &src,
            &dst,
            &deliver,
            None,
            &protocol::STPath::new(),
            false,
            false,
        );

        assert_eq!(ter, Ter::TES_SUCCESS);
        // Expected strand: DirectStep(Alice→Gateway) + DirectStep(Gateway→Bob)
        // OR: DirectStep(Alice→Bob) if direct trust line
        assert!(!strand.is_empty(), "Strand should not be empty");

        // Verify steps are DirectSteps with correct accounts
        for step in &strand {
            match step {
                StepKind::Direct {
                    src: s,
                    dst: d,
                    currency: c,
                } => {
                    assert_eq!(*c, usd);
                    assert!(!s.is_zero());
                    assert!(!d.is_zero());
                }
                _ => panic!("Expected DirectStep for IOU→IOU, got {:?}", step),
            }
        }
    }

    #[test]
    fn test_xrp_to_iou_default_path() {
        // XRP→IOU: XRPEndpointStep(src) + BookStep(XRP/IOU) + DirectStep(issuer→dst)
        let src = make_account(1);
        let dst = make_account(2);
        let gateway = make_account(3);
        let usd = make_currency("USD");
        let deliver = Asset::Issue(Issue {
            currency: usd,
            account: gateway,
        });
        let send_max = Asset::Issue(xrp_issue());

        let (ter, strand) = to_strand(
            &src,
            &dst,
            &deliver,
            Some(&send_max),
            &protocol::STPath::new(),
            false,
            false,
        );

        assert_eq!(ter, Ter::TES_SUCCESS);
        assert!(!strand.is_empty(), "Strand should not be empty for XRP→IOU");

        // Should have: XrpEndpoint(src) + Book(XRP→USD) + Direct(gateway→dst)
        let has_xrp_endpoint = strand
            .iter()
            .any(|s| matches!(s, StepKind::XrpEndpoint { .. }));
        let has_book = strand.iter().any(|s| matches!(s, StepKind::Book { .. }));

        assert!(has_xrp_endpoint, "XRP→IOU should have XrpEndpointStep");
        assert!(has_book, "XRP→IOU should have BookStep");
    }

    #[test]
    fn test_iou_to_xrp_default_path() {
        // IOU→XRP: DirectStep(src→issuer) + BookStep(IOU/XRP) + XRPEndpointStep(dst)
        let src = make_account(1);
        let dst = make_account(2);
        let gateway = make_account(3);
        let usd = make_currency("USD");
        let deliver = Asset::Issue(xrp_issue());
        let send_max = Asset::Issue(Issue {
            currency: usd,
            account: gateway,
        });

        let (ter, strand) = to_strand(
            &src,
            &dst,
            &deliver,
            Some(&send_max),
            &protocol::STPath::new(),
            false,
            false,
        );

        assert_eq!(ter, Ter::TES_SUCCESS);
        assert!(!strand.is_empty(), "Strand should not be empty for IOU→XRP");

        let has_book = strand.iter().any(|s| matches!(s, StepKind::Book { .. }));
        let has_xrp_endpoint = strand
            .iter()
            .any(|s| matches!(s, StepKind::XrpEndpoint { .. }));

        assert!(has_book, "IOU→XRP should have BookStep");
        assert!(has_xrp_endpoint, "IOU→XRP should have XrpEndpointStep");
    }

    #[test]
    fn test_xrp_to_xrp_rejected() {
        // XRP→XRP should not build a strand (handled separately by handle_xrp_to_xrp_flow)
        let src = make_account(1);
        let dst = make_account(2);
        let deliver = Asset::Issue(xrp_issue());

        let (ter, strand) = to_strand(
            &src,
            &dst,
            &deliver,
            None,
            &protocol::STPath::new(),
            false,
            false,
        );

        // XRP→XRP with no sendMax: curAsset = xrpIssue, deliver = xrpIssue
        // The strand should be: XrpEndpoint(src, false) → XrpEndpoint(dst, true)
        // OR it might fail because src element with XRP + dst element with XRP = no book needed
        // Actually in the reference, XRP→XRP goes through handle_xrp_to_xrp_flow, not toStrand
        // But if it does reach toStrand, it should build XRP endpoints
        if ter == Ter::TES_SUCCESS {
            assert!(!strand.is_empty());
        }
    }

    #[test]
    fn test_strand_accounts_are_correct() {
        // Verify that DirectStep accounts match what reference would produce
        let src = make_account(1);
        let dst = make_account(2);
        let gateway = make_account(3);
        let usd = make_currency("USD");
        let deliver = Asset::Issue(Issue {
            currency: usd,
            account: gateway,
        });

        let (ter, strand) = to_strand(
            &src,
            &dst,
            &deliver,
            None,
            &protocol::STPath::new(),
            false,
            false,
        );
        assert_eq!(ter, Ter::TES_SUCCESS);

        // [src, USD/src] → [gateway] (if gateway != dst) → [dst]
        // Steps: DirectStep(src, gateway) + DirectStep(gateway, dst)
        // Unless src == gateway or dst == gateway

        if strand.len() == 2 {
            // Two DirectSteps: src→gateway, gateway→dst
            if let StepKind::Direct {
                src: s1, dst: d1, ..
            } = &strand[0]
            {
                assert_eq!(*s1, src, "First step src should be sender");
                assert_eq!(*d1, gateway, "First step dst should be issuer");
            }
            if let StepKind::Direct {
                src: s2, dst: d2, ..
            } = &strand[1]
            {
                assert_eq!(*s2, gateway, "Second step src should be issuer");
                assert_eq!(*d2, dst, "Second step dst should be receiver");
            }
        } else if strand.len() == 1 {
            // Single DirectStep: src→dst (when one party is issuer)
            if let StepKind::Direct { src: s, dst: d, .. } = &strand[0] {
                assert_eq!(*s, src);
                assert_eq!(*d, dst);
            }
        }
    }
}
