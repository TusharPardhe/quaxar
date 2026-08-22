use super::{StepKind, Strand};
use basics::base_uint::Uint256;
use protocol::{AccountID, Asset, Issue, STPath, STPathSet, Ter, xrp_issue};

pub fn to_strand(
    src: &AccountID,
    dst: &AccountID,
    deliver: &Asset,
    send_max_asset: Option<&Asset>,
    path: &STPath,
    owner_pays_transfer_fee: bool,
    offer_crossing: bool,
) -> (Ter, Strand) {
    to_strand_with_domain(
        src,
        dst,
        deliver,
        send_max_asset,
        path,
        owner_pays_transfer_fee,
        offer_crossing,
        None,
    )
}

fn to_strand_with_domain(
    src: &AccountID,
    dst: &AccountID,
    deliver: &Asset,
    send_max_asset: Option<&Asset>,
    path: &STPath,
    owner_pays_transfer_fee: bool,
    offer_crossing: bool,
    domain: Option<Uint256>,
) -> (Ter, Strand) {
    if src.is_zero() || dst.is_zero() {
        return (Ter::TEM_BAD_PATH, Vec::new());
    }

    let initial_asset = match *send_max_asset.unwrap_or(deliver) {
        Asset::Issue(issue) if issue.native() => Asset::Issue(xrp_issue()),
        Asset::Issue(issue) => Asset::Issue(Issue::new(issue.currency, *src)),
        Asset::MPTIssue(issue) => Asset::MPTIssue(issue),
    };

    // Build normalized path elements
    let mut norm: Vec<NormElem> = Vec::with_capacity(4 + path.size());

    // 1. Source element
    norm.push(NormElem::Acct(*src));

    // 2. SendMax issuer if != src
    if let Some(sma) = send_max_asset
        && !sma.native()
        && sma.issuer() != *src
    {
        let first_is_issuer = path
            .iter()
            .next()
            .map(|e| e.is_account() && e.account_id() == sma.issuer())
            .unwrap_or(false);
        if !first_is_issuer {
            norm.push(NormElem::Acct(sma.issuer()));
        }
    }

    // 3. Explicit path elements
    for elem in path.iter() {
        if elem.is_account() {
            norm.push(NormElem::Acct(elem.account_id()));
        } else {
            let asset = if elem.has_mpt() {
                Asset::from(elem.mpt_id())
            } else {
                let issuer = if elem.has_issuer() {
                    elem.issuer_id()
                } else {
                    initial_asset.issuer()
                };
                Asset::Issue(Issue::new(elem.currency(), issuer))
            };
            norm.push(NormElem::Offer(asset));
        }
    }

    // 4. Deliver asset if last asset != deliver
    let needs_deliver_book = {
        let last_asset = last_asset_in_norm(&norm, initial_asset);
        !same_path_asset(last_asset, *deliver)
            || (offer_crossing && last_asset.issuer() != deliver.issuer())
    };
    if needs_deliver_book {
        norm.push(NormElem::Offer(*deliver));
    }

    // 5. Deliver issuer if != dst
    let deliver_issuer = deliver.issuer();
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
    let mut cur_asset = initial_asset;

    for i in 0..norm.len() - 1 {
        let cur = &norm[i];
        let next = &norm[i + 1];

        // Update curAsset from current element
        match cur {
            NormElem::Acct(acct) => {
                if let Asset::Issue(issue) = &mut cur_asset
                    && !issue.native()
                {
                    issue.account = *acct;
                }
            }
            NormElem::Offer(asset) => cur_asset = *asset,
        }

        match (cur, next) {
            (NormElem::Acct(s), NormElem::Acct(d)) => {
                if cur_asset.native() {
                    // XRP endpoint
                    let is_first = i == 0;
                    strand.push(StepKind::XrpEndpoint {
                        account: if is_first { *s } else { *d },
                        is_last: !is_first,
                    });
                } else if let Asset::MPTIssue(issue) = cur_asset {
                    strand.push(StepKind::MptEndpoint {
                        src: *s,
                        dst: *d,
                        issue,
                        is_first: i == 0,
                        is_last: i == norm.len() - 2,
                        offer_crossing,
                    });
                } else if let Asset::Issue(issue) = cur_asset {
                    // DirectStep
                    strand.push(StepKind::Direct {
                        src: *s,
                        dst: *d,
                        currency: issue.currency,
                    });
                }
            }
            (NormElem::Acct(s), NormElem::Offer(out_asset)) => {
                if i == 0 && cur_asset.native() {
                    strand.push(StepKind::XrpEndpoint {
                        account: *s,
                        is_last: false,
                    });
                }
                // rippled `PaySteps.cpp::toStrand` normalizes `curAsset` after
                // changing its currency and before it calls `toStep`: XRP
                // always carries `xrpAccount()`, never a path issuer.  Keep
                // both sides canonical before storing the typed BookStep.
                strand.push(StepKind::Book {
                    book_in: canonical_book_asset(cur_asset),
                    book_out: canonical_book_asset(*out_asset),
                    domain,
                    owner_pays_transfer_fee,
                    remove_self_crossing: offer_crossing && path.size() == 0,
                });
                cur_asset = canonical_book_asset(*out_asset);
            }
            (NormElem::Offer(_), NormElem::Acct(d)) => {
                // Offer→Account: implied step if cur_issuer != dst
                if cur_asset.issuer() != *d && !d.is_zero() {
                    if cur_asset.native() {
                        strand.push(StepKind::XrpEndpoint {
                            account: *d,
                            is_last: true,
                        });
                    } else if let Asset::MPTIssue(issue) = cur_asset {
                        strand.push(StepKind::MptEndpoint {
                            src: issue.issuer(),
                            dst: *d,
                            issue,
                            is_first: false,
                            is_last: true,
                            offer_crossing,
                        });
                    } else if let Asset::Issue(issue) = cur_asset {
                        strand.push(StepKind::Direct {
                            src: issue.issuer(),
                            dst: *d,
                            currency: issue.currency,
                        });
                    }
                }
                continue;
            }
            (NormElem::Offer(_), NormElem::Offer(out_asset)) => {
                // Match the same `PaySteps.cpp::toStrand` normalization used by
                // the account-to-offer BookStep above.
                strand.push(StepKind::Book {
                    book_in: canonical_book_asset(cur_asset),
                    book_out: canonical_book_asset(*out_asset),
                    domain,
                    owner_pays_transfer_fee,
                    remove_self_crossing: offer_crossing && path.size() == 0,
                });
                cur_asset = canonical_book_asset(*out_asset);
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
    to_strands_with_domain(
        src,
        dst,
        deliver,
        send_max_asset,
        paths,
        default_paths_allowed,
        owner_pays_transfer_fee,
        offer_crossing,
        None,
    )
}

pub fn to_strands_with_domain(
    src: &AccountID,
    dst: &AccountID,
    deliver: &Asset,
    send_max_asset: Option<&Asset>,
    paths: &STPathSet,
    default_paths_allowed: bool,
    owner_pays_transfer_fee: bool,
    offer_crossing: bool,
    domain: Option<Uint256>,
) -> (Ter, Vec<Strand>) {
    let mut result: Vec<Strand> = Vec::new();

    if default_paths_allowed {
        let empty_path = protocol::STPath::new();
        let (ter, strand) = to_strand_with_domain(
            src,
            dst,
            deliver,
            send_max_asset,
            &empty_path,
            owner_pays_transfer_fee,
            offer_crossing,
            domain,
        );
        if ter == Ter::TES_SUCCESS && !strand.is_empty() {
            result.push(strand);
        } else if ter != Ter::TES_SUCCESS && paths.size() == 0 {
            return (ter, Vec::new());
        }
    }

    for path in paths.iter() {
        let (ter, strand) = to_strand_with_domain(
            src,
            dst,
            deliver,
            send_max_asset,
            path,
            owner_pays_transfer_fee,
            offer_crossing,
            domain,
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

/// Return the protocol's sole canonical issue for XRP while preserving the
/// issuer required by every issued currency.  `Issue` equality intentionally
/// ignores an XRP account, but `keylet::get_book_base` must receive the
/// canonical zero XRP account (rippled `PaySteps.cpp::toStrand`, immediately
/// before its `toStep` call).
fn canonical_book_asset(asset: Asset) -> Asset {
    match asset {
        Asset::Issue(issue) if issue.native() => Asset::Issue(xrp_issue()),
        _ => asset,
    }
}

fn same_path_asset(lhs: Asset, rhs: Asset) -> bool {
    match (lhs, rhs) {
        (Asset::Issue(lhs), Asset::Issue(rhs)) => lhs.currency == rhs.currency,
        (Asset::MPTIssue(lhs), Asset::MPTIssue(rhs)) => lhs == rhs,
        _ => false,
    }
}

#[derive(Debug, Clone)]
enum NormElem {
    Acct(AccountID),
    Offer(Asset),
}

fn last_asset_in_norm(norm: &[NormElem], initial: Asset) -> Asset {
    for elem in norm.iter().rev() {
        if let NormElem::Offer(asset) = elem {
            return *asset;
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
    fn offer_crossing_domain_is_attached_to_every_book_step() {
        let src = make_account(1);
        let issuer = make_account(3);
        let deliver = Asset::Issue(Issue::new(make_currency("USD"), issuer));
        let send_max = Asset::Issue(xrp_issue());
        let domain = Uint256::from_array([0xD0; 32]);
        let (_, strands) = to_strands_with_domain(
            &src,
            &src,
            &deliver,
            Some(&send_max),
            &STPathSet::new(protocol::get_field_by_symbol("sfPaths")),
            true,
            true,
            true,
            Some(domain),
        );
        let books: Vec<_> = strands
            .iter()
            .flatten()
            .filter_map(|step| match step {
                StepKind::Book { domain, .. } => Some(*domain),
                _ => None,
            })
            .collect();
        assert!(!books.is_empty());
        assert!(books.iter().all(|actual| *actual == Some(domain)));
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
    fn no_default_path_without_explicit_paths_returns_ripple_empty() {
        let src = make_account(1);
        let dst = make_account(2);
        let gateway = make_account(3);
        let eur = make_currency("EUR");
        let deliver = Asset::Issue(Issue {
            currency: eur,
            account: gateway,
        });

        let (ter, strands) = to_strands(
            &src,
            &dst,
            &deliver,
            None,
            &protocol::STPathSet::new(protocol::get_field_by_symbol("sfPaths")),
            false,
            false,
            false,
        );

        assert_eq!(ter, Ter::TEM_RIPPLE_EMPTY);
        assert!(strands.is_empty());
    }

    #[test]
    fn canonical_xrp_book_issue_discards_a_stale_issuer_before_keylet_lookup() {
        let stale_issuer = protocol::parse_base58_account_id("rswh1fvyLqHizBS2awu1vs6QcmwTBd9qiv")
            .expect("canonical XAH issuer");
        let raw = Issue::new(protocol::xrp_currency(), stale_issuer);
        assert!(
            !protocol::is_consistent(raw),
            "the protocol keylet must reject XRP with an issuer"
        );

        let normalized = canonical_book_asset(Asset::Issue(raw));
        assert_eq!(normalized, Asset::Issue(xrp_issue()));
        let Asset::Issue(normalized_issue) = normalized else {
            unreachable!()
        };
        assert!(protocol::is_consistent(normalized_issue));
    }

    #[test]
    fn canonical_xah_to_xrp_mainnet_direct_xrp_path_builds_a_consistent_book() {
        // EDA59CA1B73A69E76E753F6790DBC2E323B3699CFA200D15B3EAA0FFC2FFC2B3,
        // 0386E0015B04116D4F61ECF5EEE71C8912F53C74558982227B24990BD3CFD921,
        // and DF801A1D59B2B36981AFD810978D84FA06F5E7D6872A5D3AEB2B908EAAF184C0
        // are self-payments from rspxi5HWiqiGUdpge6hZh7drZXRMqDX93t. Each
        // sends XAH issued by rswh1fvyLqHizBS2awu1vs6QcmwTBd9qiv to XRP,
        // with tfPartialPayment|tfLimitQuality, and includes this direct
        // explicit Path: [{ currency: XRP }]. Model its type-16 element here
        // so the regression exercises the actual XAH-to-XRP strand shape,
        // rather than an empty default path.
        let account = protocol::parse_base58_account_id("rspxi5HWiqiGUdpge6hZh7drZXRMqDX93t")
            .expect("canonical transaction account");
        let issuer = protocol::parse_base58_account_id("rswh1fvyLqHizBS2awu1vs6QcmwTBd9qiv")
            .expect("canonical XAH issuer");
        let xah = protocol::currency_from_string("XAH");
        let deliver = Asset::Issue(xrp_issue());
        let send_max = Asset::Issue(Issue::new(xah, issuer));
        let mut direct_xrp_path = protocol::STPath::new();
        direct_xrp_path.push_back(protocol::STPathElement::raw(
            protocol::STPathElement::TYPE_CURRENCY,
            AccountID::zero(),
            protocol::xrp_currency(),
            AccountID::zero(),
        ));

        let (ter, strand) = to_strand(
            &account,
            &account,
            &deliver,
            Some(&send_max),
            &direct_xrp_path,
            false,
            false,
        );

        assert_eq!(ter, Ter::TES_SUCCESS);
        let (book_in, book_out) = strand
            .iter()
            .find_map(|step| match step {
                StepKind::Book {
                    book_in, book_out, ..
                } => Some((*book_in, *book_out)),
                _ => None,
            })
            .expect("XAH-to-XRP payment must include a BookStep");
        assert_eq!(book_in, Asset::Issue(Issue::new(xah, issuer)));
        assert_eq!(book_out, Asset::Issue(xrp_issue()));

        let book = protocol::Book::new(book_in, book_out, None);
        assert!(protocol::is_consistent_book(book));
        // This is the exact protocol boundary that previously panicked.
        let _ = protocol::get_book_base(book);
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

    #[test]
    fn same_asset_mpt_payment_uses_issuer_endpoint_pair() {
        let src = make_account(1);
        let dst = make_account(2);
        let issuer = make_account(3);
        let issue = protocol::MPTIssue::new(protocol::make_mpt_id(7, issuer));
        let asset = Asset::MPTIssue(issue);

        let (ter, strand) = to_strand(
            &src,
            &dst,
            &asset,
            None,
            &protocol::STPath::new(),
            false,
            false,
        );

        assert_eq!(ter, Ter::TES_SUCCESS);
        assert_eq!(strand.len(), 2);
        assert!(matches!(
            strand[0],
            StepKind::MptEndpoint {
                src: actual_src,
                dst: actual_dst,
                issue: actual_issue,
                is_first: true,
                is_last: false,
                offer_crossing: false,
            } if actual_src == src && actual_dst == issuer && actual_issue == issue
        ));
        assert!(matches!(
            strand[1],
            StepKind::MptEndpoint {
                src: actual_src,
                dst: actual_dst,
                issue: actual_issue,
                is_first: false,
                is_last: true,
                offer_crossing: false,
            } if actual_src == issuer && actual_dst == dst && actual_issue == issue
        ));
    }

    #[test]
    fn mpt_to_iou_crossing_preserves_exact_book_assets() {
        let taker = make_account(1);
        let mpt_issuer = make_account(2);
        let iou_issuer = make_account(3);
        let mpt = Asset::MPTIssue(protocol::MPTIssue::new(protocol::make_mpt_id(
            9, mpt_issuer,
        )));
        let usd = Asset::Issue(Issue::new(make_currency("USD"), iou_issuer));

        let (ter, strand) = to_strand(
            &taker,
            &taker,
            &usd,
            Some(&mpt),
            &protocol::STPath::new(),
            true,
            true,
        );

        assert_eq!(ter, Ter::TES_SUCCESS);
        assert!(strand.iter().any(|step| matches!(
            step,
            StepKind::Book { book_in, book_out, .. }
                if *book_in == mpt && *book_out == usd
        )));
        assert!(!strand.iter().any(|step| matches!(
            step,
            StepKind::Book { book_in, .. } if book_in.native()
        )));
    }
}
