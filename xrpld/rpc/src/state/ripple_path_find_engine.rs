//! Ledger-backed `ripple_path_find` engine.
//!
//! Candidate discovery mirrors rippled's costed path table. Every candidate is
//! then evaluated in an isolated `PaymentSandbox`; a syntactically plausible
//! path is never returned unless the real Flow/RippleCalc engine can fund it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use app::ApplicationRoot;
use app::paths::PathFinderRequest;
use app::paths::PathFinderResult;
use basics::base_uint::Uint160;
use ledger::{Ledger, PaymentSandbox};
use protocol::{
    AccountID, ApplyFlags, Asset, Issue, JsonOptions, JsonValue, LedgerEntryType, PathAsset,
    STAmount, STPath, STPathElement, STPathSet, StBase, Ter, equal_tokens, get_field_by_symbol,
    get_rate, is_tes_success, owner_dir_keylet, page_keylet, to_max_amount, xrp_account, xrp_issue,
};

const MAX_COMPLETE_PATHS: usize = 1_000;
const MAX_PATHS: usize = 4;
const MAX_AUTO_SOURCE_ASSETS: usize = 88;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKind {
    Source,
    Accounts,
    Books,
    XrpBook,
    DestBook,
    Destination,
}

#[derive(Debug, Clone)]
struct TrustEdge {
    peer: AccountID,
    currency: protocol::Currency,
    balance: STAmount,
    peer_limit: STAmount,
    auth: bool,
    no_ripple: bool,
    no_ripple_peer: bool,
    freeze_peer: bool,
}

#[derive(Debug, Clone)]
struct RankedPath {
    path: STPath,
    quality: u64,
    liquidity: STAmount,
    ordinal: usize,
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn asset_issuer(asset: Asset) -> AccountID {
    asset.issuer()
}

fn same_token(left: Asset, right: Asset) -> bool {
    left.token() == right.token()
}

fn path_asset(asset: Asset) -> PathAsset {
    PathAsset::from(asset)
}

fn path_end(path: &STPath, source: &STPathElement) -> STPathElement {
    path.back().cloned().unwrap_or_else(|| source.clone())
}

fn append_unique(paths: &mut Vec<STPath>, path: STPath) {
    if paths.len() < MAX_COMPLETE_PATHS && !paths.contains(&path) {
        paths.push(path);
    }
}

fn append_partial(paths: &mut Vec<STPath>, base: &STPath, element: STPathElement) {
    let mut path = base.clone();
    path.push_back(element);
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn owner_entries(ledger: &Ledger, account: AccountID) -> Vec<protocol::STLedgerEntry> {
    let root = owner_dir_keylet(Uint160::from_void(account.data()));
    let mut entries = Vec::new();
    let mut page = 0u64;
    loop {
        let keylet = if page == 0 {
            root
        } else {
            page_keylet(root, page)
        };
        let Ok(Some(directory)) = ledger.read(keylet) else {
            break;
        };
        for index in directory.get_field_v256(sf("sfIndexes")).value() {
            let keylet = protocol::unchecked_keylet(*index);
            if let Ok(Some(entry)) = ledger.read(keylet) {
                entries.push(entry);
            }
        }
        page = directory.get_field_u64(sf("sfIndexNext"));
        if page == 0 {
            break;
        }
    }
    entries
}

fn trust_edges(ledger: &Ledger, account: AccountID) -> Vec<TrustEdge> {
    let mut edges = Vec::new();
    for entry in owner_entries(ledger, account) {
        if entry.get_type() != LedgerEntryType::RippleState {
            continue;
        }
        let low = entry.get_field_amount(sf("sfLowLimit"));
        let high = entry.get_field_amount(sf("sfHighLimit"));
        let low_account = low.issue().account;
        let high_account = high.issue().account;
        if account != low_account && account != high_account {
            continue;
        }
        let view_low = account == low_account;
        let mut balance = entry.get_field_amount(sf("sfBalance"));
        if !view_low {
            balance.negate();
        }
        let flags = entry.get_field_u32(sf("sfFlags"));
        let side = |low_flag, high_flag| if view_low { low_flag } else { high_flag };
        let peer_side = |low_flag, high_flag| if view_low { high_flag } else { low_flag };
        edges.push(TrustEdge {
            peer: if account == low_account {
                high_account
            } else {
                low_account
            },
            currency: low.issue().currency,
            balance,
            peer_limit: if view_low { high } else { low },
            auth: flags & side(0x0001_0000, 0x0002_0000) != 0,
            no_ripple: flags & side(0x0010_0000, 0x0020_0000) != 0,
            no_ripple_peer: flags & peer_side(0x0010_0000, 0x0020_0000) != 0,
            freeze_peer: flags & peer_side(0x0040_0000, 0x0080_0000) != 0,
        });
    }
    edges.sort_by(|a, b| b.peer.cmp(&a.peer).then(a.currency.cmp(&b.currency)));
    edges.dedup_by(|a, b| a.peer == b.peer && a.currency == b.currency);
    edges
}

fn mpt_is_maxed(ledger: &Ledger, id: protocol::MPTID) -> bool {
    let Ok(Some(issuance)) = ledger.read(protocol::mpt_issuance_keylet_from_mptid(id)) else {
        return true;
    };
    issuance.get_field_u64(sf("sfOutstandingAmount")) as i64
        == ledger::mptoken_helpers::max_mpt_amount(&issuance)
}

fn account_mpt_state(
    ledger: &Ledger,
    account: AccountID,
    id: protocol::MPTID,
) -> Option<(bool, bool)> {
    for entry in owner_entries(ledger, account) {
        match entry.get_type() {
            LedgerEntryType::MPToken if entry.get_field_h192(sf("sfMPTokenIssuanceID")) == id => {
                return Some((
                    entry.get_field_u64(sf("sfMPTAmount")) == 0,
                    mpt_is_maxed(ledger, id),
                ));
            }
            LedgerEntryType::MPTokenIssuance
                if protocol::make_mpt_id(entry.get_field_u32(sf("sfSequence")), account) == id =>
            {
                return Some((false, mpt_is_maxed(ledger, id)));
            }
            _ => {}
        }
    }
    None
}

fn insert_auto_source_asset(
    assets: &mut BTreeSet<Asset>,
    asset: Asset,
) -> Result<(), crate::RpcStatus> {
    if !assets.contains(&asset) && assets.len() >= MAX_AUTO_SOURCE_ASSETS {
        return Err(crate::RpcStatus::new(crate::RpcErrorCode::Internal));
    }
    assets.insert(asset);
    Ok(())
}

fn source_assets(
    ledger: &Ledger,
    request: &PathFinderRequest,
) -> Result<Vec<Asset>, crate::RpcStatus> {
    if !request.source_assets.is_empty() {
        return Ok(request.source_assets.clone());
    }
    if let Some(send_max) = request.parsed_send_max.as_ref() {
        return Ok(vec![send_max.asset()]);
    }

    let mut assets = BTreeSet::from([Asset::Issue(xrp_issue())]);
    for entry in owner_entries(ledger, request.source_account_id) {
        match entry.get_type() {
            LedgerEntryType::RippleState => {
                let low = entry.get_field_amount(sf("sfLowLimit"));
                let high = entry.get_field_amount(sf("sfHighLimit"));
                let balance = entry.get_field_amount(sf("sfBalance"));
                let source_is_low = low.issue().account == request.source_account_id;
                let peer_limit = if source_is_low { &high } else { &low };
                let can_send = if source_is_low {
                    let mut negated = balance.clone();
                    negated.negate();
                    balance.signum() > 0 || negated < *peer_limit
                } else {
                    balance.signum() < 0 || balance < peer_limit.clone()
                };
                if can_send {
                    insert_auto_source_asset(
                        &mut assets,
                        Asset::Issue(Issue::new(low.issue().currency, request.source_account_id)),
                    )?;
                }
            }
            LedgerEntryType::MPToken => {
                let amount = entry.get_field_u64(sf("sfMPTAmount"));
                let id = entry.get_field_h192(sf("sfMPTokenIssuanceID"));
                if amount > 0 && !mpt_is_maxed(ledger, id) {
                    insert_auto_source_asset(
                        &mut assets,
                        Asset::from(protocol::MPTIssue::new(id)),
                    )?;
                }
            }
            LedgerEntryType::MPTokenIssuance => {
                let id = protocol::make_mpt_id(
                    entry.get_field_u32(sf("sfSequence")),
                    request.source_account_id,
                );
                if !mpt_is_maxed(ledger, id) {
                    insert_auto_source_asset(
                        &mut assets,
                        Asset::from(protocol::MPTIssue::new(id)),
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(assets.into_iter().collect())
}

fn templates(source: Asset, destination: Asset) -> &'static [(u32, &'static [NodeKind])] {
    use NodeKind::{
        Accounts as A, Books as B, DestBook as F, Destination as D, Source as S, XrpBook as X,
    };
    const XRP_IOU: &[(u32, &[NodeKind])] = &[
        (1, &[S, F, D]),
        (3, &[S, F, A, D]),
        (5, &[S, F, A, A, D]),
        (6, &[S, B, F, D]),
        (8, &[S, B, A, F, D]),
        (9, &[S, B, F, A, D]),
        (10, &[S, B, A, F, A, D]),
    ];
    const IOU_XRP: &[(u32, &[NodeKind])] = &[
        (1, &[S, X, D]),
        (2, &[S, A, X, D]),
        (6, &[S, A, A, X, D]),
        (7, &[S, B, X, D]),
        (8, &[S, A, B, X, D]),
        (9, &[S, A, B, A, X, D]),
    ];
    const SAME: &[(u32, &[NodeKind])] = &[
        (1, &[S, A, D]),
        (1, &[S, F, D]),
        (4, &[S, A, F, D]),
        (4, &[S, F, A, D]),
        (5, &[S, A, A, D]),
        (5, &[S, B, F, D]),
        (6, &[S, X, F, A, D]),
        (6, &[S, A, F, A, D]),
        (6, &[S, A, X, F, D]),
        (6, &[S, A, X, F, A, D]),
        (6, &[S, A, B, F, D]),
        (7, &[S, A, A, A, D]),
    ];
    const CROSS: &[(u32, &[NodeKind])] = &[
        (1, &[S, F, A, D]),
        (1, &[S, A, F, D]),
        (3, &[S, A, F, A, D]),
        (4, &[S, X, F, D]),
        (5, &[S, A, X, F, D]),
        (5, &[S, X, F, A, D]),
        (5, &[S, B, F, D]),
        (6, &[S, A, X, F, A, D]),
        (6, &[S, A, B, F, D]),
        (7, &[S, A, A, F, D]),
        (8, &[S, A, A, F, A, D]),
        (9, &[S, A, F, A, A, D]),
    ];
    if source.native() && destination.native() {
        &[]
    } else if source.native() {
        XRP_IOU
    } else if destination.native() {
        IOU_XRP
    } else if same_token(source, destination) {
        SAME
    } else {
        CROSS
    }
}

struct Discovery<'a> {
    ledger: &'a Ledger,
    app: &'a ApplicationRoot,
    source_account: AccountID,
    destination_account: AccountID,
    destination_asset: Asset,
    effective_destination: AccountID,
    source_asset: Asset,
    source_element: STPathElement,
    domain: Option<protocol::Domain>,
    complete: Vec<STPath>,
}

impl Discovery<'_> {
    fn no_ripple_out(&self, current: &STPath) -> bool {
        let Some(end) = current.back() else {
            return false;
        };
        if !end.is_account() || !end.has_currency() {
            return false;
        }
        let from = if current.size() == 1 {
            self.source_account
        } else {
            current[current.size() - 2].account_id()
        };
        trust_edges(self.ledger, end.account_id())
            .into_iter()
            .find(|edge| edge.peer == from && edge.currency == end.currency())
            .is_some_and(|edge| edge.no_ripple)
    }

    fn paths_out(&self, account: AccountID, asset: PathAsset, incoming: bool) -> i32 {
        let Some(root) = self
            .ledger
            .read(protocol::account_keylet(Uint160::from_void(account.data())))
            .ok()
            .flatten()
        else {
            return 0;
        };
        let input = asset.visit(
            |currency| Asset::Issue(Issue::new(currency, account)),
            |id| Asset::from(protocol::MPTIssue::new(id)),
        );
        let frozen = match input {
            Asset::Issue(_) => root.is_flag(protocol::lsfGlobalFreeze),
            Asset::MPTIssue(issue) => {
                ledger::mptoken_helpers::is_global_frozen_mpt(self.ledger, &issue).unwrap_or(true)
            }
        };
        if frozen {
            return 0;
        }
        let mut count = self
            .app
            .order_book_db()
            .get_book_size_asset(input, self.domain);
        if asset.holds_currency() {
            for edge in trust_edges(self.ledger, account) {
                if edge.currency != asset.currency() || (incoming && edge.no_ripple) {
                    continue;
                }
                let mut negated = edge.balance.clone();
                negated.negate();
                let can_leave = edge.balance.signum() > 0
                    || (edge.peer_limit.signum() > 0 && negated < edge.peer_limit);
                if !can_leave || edge.no_ripple_peer || edge.freeze_peer {
                    continue;
                }
                if edge.peer == self.effective_destination
                    && PathAsset::from(self.destination_asset) == asset
                {
                    count += 10_000;
                } else {
                    count += 1;
                }
            }
        } else {
            let id = asset.mpt_id();
            let issue = protocol::MPTIssue::new(id);
            let requires_auth = !is_tes_success(
                ledger::mptoken_helpers::require_auth_mpt(self.ledger, &issue, &account)
                    .unwrap_or(Ter::TEC_NO_AUTH),
            );
            if let Some((zero_balance, maxed_out)) = account_mpt_state(self.ledger, account, id)
                && !zero_balance
                && !maxed_out
                && !requires_auth
                && !ledger::mptoken_helpers::is_individual_frozen_mpt(self.ledger, &account, &issue)
                    .unwrap_or(true)
            {
                if account == self.effective_destination
                    && PathAsset::from(self.destination_asset) == asset
                {
                    count += 10_000;
                } else {
                    count += 1;
                }
            }
        }
        count
    }

    fn add_accounts(&mut self, current: &STPath, destination_only: bool, out: &mut Vec<STPath>) {
        let end = path_end(current, &self.source_element);
        let asset = end.path_asset();
        if asset.is_xrp() {
            if self.destination_asset.native() && !current.empty() {
                append_unique(&mut self.complete, current.clone());
            }
            return;
        }
        if asset.holds_mpt() {
            let id = asset.mpt_id();
            let Some((zero_balance, maxed_out)) =
                account_mpt_state(self.ledger, end.account_id(), id)
            else {
                return;
            };
            if zero_balance || maxed_out {
                return;
            }
            let issuer = protocol::MPTIssue::new(id).issuer();
            if destination_only && issuer != self.effective_destination {
                return;
            }
            if issuer == self.effective_destination
                && PathAsset::from(self.destination_asset) == asset
            {
                if !current.empty() {
                    append_unique(&mut self.complete, current.clone());
                }
            } else if issuer != self.source_account
                && !current.has_seen(issuer, asset, issuer)
                && self.paths_out(issuer, asset, false) != 0
            {
                append_partial(
                    out,
                    current,
                    STPathElement::raw(STPathElement::TYPE_ACCOUNT, issuer, asset, issuer),
                );
            }
            return;
        }
        let Some(end_root) = self
            .ledger
            .read(protocol::account_keylet(Uint160::from_void(
                end.account_id().data(),
            )))
            .ok()
            .flatten()
        else {
            return;
        };
        let require_auth = end_root.is_flag(protocol::lsfRequireAuth);
        let no_ripple_out = self.no_ripple_out(current);
        let mut candidates = trust_edges(self.ledger, end.account_id())
            .into_iter()
            .filter(|edge| asset.holds_currency() && edge.currency == asset.currency())
            .filter(|edge| !destination_only || edge.peer == self.effective_destination)
            .filter_map(|edge| {
                let mut negated = edge.balance.clone();
                negated.negate();
                let unusable = (edge.balance.signum() <= 0
                    && (edge.peer_limit.signum() == 0
                        || negated >= edge.peer_limit
                        || (require_auth && !edge.auth)))
                    || (no_ripple_out && edge.no_ripple);
                if unusable {
                    return None;
                }
                let priority = if edge.peer == self.effective_destination {
                    10_000
                } else {
                    self.paths_out(edge.peer, asset, edge.no_ripple_peer)
                };
                (priority != 0).then_some((priority, edge))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|(left_priority, left), (right_priority, right)| {
            right_priority
                .cmp(left_priority)
                .then_with(|| right.peer.cmp(&left.peer))
        });
        candidates.truncate(if end.account_id() == self.source_account {
            50
        } else {
            10
        });
        for (_, edge) in candidates {
            if self.effective_destination != self.destination_account
                && edge.peer == self.destination_account
            {
                continue;
            }
            if current.has_seen(edge.peer, asset, edge.peer) || edge.peer == self.source_account {
                continue;
            }
            if edge.peer == self.effective_destination
                && PathAsset::from(self.destination_asset) == asset
            {
                if !current.empty() {
                    append_unique(&mut self.complete, current.clone());
                }
                continue;
            }
            append_partial(
                out,
                current,
                STPathElement::raw(STPathElement::TYPE_ACCOUNT, edge.peer, asset, edge.peer),
            );
        }
    }

    fn add_books(
        &mut self,
        current: &STPath,
        xrp_only: bool,
        destination_only: bool,
        out: &mut Vec<STPath>,
    ) {
        let end = path_end(current, &self.source_element);
        let input = end.path_asset().visit(
            |currency| Asset::Issue(Issue::new(currency, end.issuer_id())),
            |id| Asset::from(protocol::MPTIssue::new(id)),
        );
        let mut books = self
            .app
            .order_book_db()
            .get_books_by_taker_pays_asset(input, self.domain);
        books.sort();
        for book in books {
            if xrp_only && !book.out.native() {
                continue;
            }
            if destination_only && !same_token(book.out, self.destination_asset) {
                continue;
            }
            let out_asset = path_asset(book.out);
            let issuer = asset_issuer(book.out);
            if current.has_seen(xrp_account(), out_asset, issuer) || book.out == self.source_asset {
                continue;
            }
            let node_type = if book.out.native() {
                STPathElement::TYPE_CURRENCY
            } else if matches!(book.out, Asset::MPTIssue(_)) {
                STPathElement::TYPE_MPT | STPathElement::TYPE_ISSUER
            } else {
                STPathElement::TYPE_CURRENCY | STPathElement::TYPE_ISSUER
            };
            let mut next = current.clone();
            let book_element = STPathElement::raw(node_type, xrp_account(), out_asset, issuer);
            if next.size() >= 2
                && next[next.size() - 1].is_account()
                && next[next.size() - 2].is_offer()
            {
                let last = next.size() - 1;
                next[last] = book_element;
            } else {
                next.push_back(book_element);
            }
            if self.effective_destination != self.destination_account
                && issuer == self.destination_account
                && same_token(book.out, self.destination_asset)
            {
                continue;
            } else if book.out.native() && self.destination_asset.native() {
                append_unique(&mut self.complete, next);
            } else if issuer == self.effective_destination
                && same_token(book.out, self.destination_asset)
            {
                append_unique(&mut self.complete, next);
            } else if !book.out.native() {
                append_partial(
                    out,
                    &next,
                    STPathElement::raw(STPathElement::TYPE_ACCOUNT, issuer, out_asset, issuer),
                );
            } else {
                append_unique(out, next);
            }
        }
    }

    fn discover(mut self, search_level: u32) -> Vec<STPath> {
        let mut cache: BTreeMap<Vec<NodeKind>, Vec<STPath>> = BTreeMap::new();
        for (cost, template) in templates(self.source_asset, self.destination_asset) {
            if *cost > search_level || self.complete.len() >= MAX_COMPLETE_PATHS {
                continue;
            }
            let mut prefix = Vec::new();
            let mut paths = Vec::new();
            for node in *template {
                prefix.push(*node);
                if let Some(existing) = cache.get(&prefix) {
                    paths = existing.clone();
                    continue;
                }
                let mut next = Vec::new();
                match node {
                    NodeKind::Source => next.push(STPath::new()),
                    NodeKind::Accounts => {
                        for path in &paths {
                            self.add_accounts(path, false, &mut next);
                        }
                    }
                    NodeKind::Destination => {
                        for path in &paths {
                            self.add_accounts(path, true, &mut next);
                        }
                    }
                    NodeKind::Books => {
                        for path in &paths {
                            self.add_books(path, false, false, &mut next);
                        }
                    }
                    NodeKind::XrpBook => {
                        for path in &paths {
                            self.add_books(path, true, false, &mut next);
                        }
                    }
                    NodeKind::DestBook => {
                        for path in &paths {
                            self.add_books(path, false, true, &mut next);
                        }
                    }
                }
                cache.insert(prefix.clone(), next.clone());
                paths = next;
            }
        }
        self.complete
    }
}

fn run_calc(
    ledger: Arc<Ledger>,
    request: &PathFinderRequest,
    source_amount: &STAmount,
    destination_amount: &STAmount,
    paths: &STPathSet,
    default_paths_allowed: bool,
    partial: bool,
) -> Option<ledger::ripple_calc::RippleCalcOutput> {
    let mut sandbox = PaymentSandbox::new(ledger, ApplyFlags::NONE);
    ledger::ripple_calc::ripple_calculate(
        &mut sandbox,
        source_amount,
        destination_amount,
        &request.destination_account_id,
        &request.source_account_id,
        paths,
        &ledger::ripple_calc::RippleCalcInput {
            partial_payment_allowed: partial,
            default_paths_allowed,
            limit_quality: false,
            is_ledger_open: false,
            domain_id: request.domain,
        },
    )
    .ok()
}

fn path_liquidity(
    ledger: Arc<Ledger>,
    request: &PathFinderRequest,
    source_amount: &STAmount,
    destination_amount: &STAmount,
    path: &STPath,
) -> Option<(u64, STAmount)> {
    let mut sandbox = PaymentSandbox::new(ledger, ApplyFlags::NONE);
    let paths = path_set([path.clone()]);
    let minimum = if request.convert_all {
        amount_for_asset(destination_amount.asset())
    } else {
        destination_amount.clone() / (MAX_PATHS as u64 + 2)
    };
    let input = |partial_payment_allowed| ledger::ripple_calc::RippleCalcInput {
        partial_payment_allowed,
        default_paths_allowed: false,
        limit_quality: false,
        is_ledger_open: false,
        domain_id: request.domain,
    };
    let first = ledger::ripple_calc::ripple_calculate(
        &mut sandbox,
        source_amount,
        &minimum,
        &request.destination_account_id,
        &request.source_account_id,
        &paths,
        &input(request.convert_all),
    )
    .ok()?;
    if !is_tes_success(first.result) || first.actual_amount_out.signum() <= 0 {
        return None;
    }

    let quality = get_rate(&first.actual_amount_out, &first.actual_amount_in);
    let mut liquidity = first.actual_amount_out;
    if !request.convert_all {
        let remaining = destination_amount.clone() - liquidity.clone();
        if remaining.signum() > 0
            && let Ok(more) = ledger::ripple_calc::ripple_calculate(
                &mut sandbox,
                source_amount,
                &remaining,
                &request.destination_account_id,
                &request.source_account_id,
                &paths,
                &input(true),
            )
            && is_tes_success(more.result)
        {
            liquidity += more.actual_amount_out;
        }
    }
    Some((quality, liquidity))
}

fn amount_for_asset(asset: Asset) -> STAmount {
    to_max_amount::<STAmount>(asset)
}

fn path_set(paths: impl IntoIterator<Item = STPath>) -> STPathSet {
    let mut set = STPathSet::new(sf("sfPaths"));
    for path in paths {
        set.push_back(path);
    }
    set
}

pub fn find_paths(
    app: &ApplicationRoot,
    ledger: Arc<Ledger>,
    request: &PathFinderRequest,
    search_level: u32,
    legacy: bool,
    previous_paths: &BTreeMap<Asset, STPathSet>,
) -> Result<PathFinderResult, crate::RpcStatus> {
    let src_key = protocol::account_keylet(Uint160::from_void(request.source_account_id.data()));
    if ledger.read(src_key).ok().flatten().is_none() {
        return Err(crate::RpcStatus::new(crate::RpcErrorCode::SrcActNotFound));
    }
    let dst_key =
        protocol::account_keylet(Uint160::from_void(request.destination_account_id.data()));
    let destination_exists = ledger.read(dst_key).ok().flatten().is_some();
    if !destination_exists && !request.parsed_destination_amount.native() {
        return Err(crate::RpcStatus::new(crate::RpcErrorCode::ActNotFound));
    }
    if !destination_exists
        && !request.convert_all
        && request.parsed_destination_amount.native()
        && request.parsed_destination_amount
            < STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(
                ledger.fees().reserve as i64,
            ))
    {
        return Err(crate::RpcStatus::new(crate::RpcErrorCode::DstAmtMalformed));
    }

    let destination_amount = if request.convert_all {
        amount_for_asset(request.parsed_destination_amount.asset())
    } else {
        request.parsed_destination_amount.clone()
    };
    let destination_asset = destination_amount.asset();
    let effective_destination = if destination_asset.native() {
        request.destination_account_id
    } else {
        destination_asset.issuer()
    };

    let mut alternatives = Vec::new();
    let mut path_context = BTreeMap::new();
    for source_asset in source_assets(&ledger, request)? {
        // rippled's path table intentionally has no XRP-to-XRP entry. A
        // direct XRP Payment needs no path-finding alternative.
        if source_asset.native() && destination_asset.native() {
            continue;
        }
        if request.source_account_id == request.destination_account_id
            && source_asset == destination_asset
        {
            continue;
        }
        let source_amount = request
            .parsed_send_max
            .as_ref()
            .filter(|amount| amount.asset() == source_asset)
            .map(|amount| {
                // RippleCalc treats a negative-one SendMax as no limit when
                // its token matches the destination and its issuer is the
                // source account. Our Flow seam takes a concrete limit, so
                // materialize rippled's unbounded sentinel as the largest
                // representable amount for the derived source asset.
                if amount.signum() < 0
                    && equal_tokens(amount.asset(), destination_asset)
                    && amount.asset().issuer() == request.source_account_id
                {
                    amount_for_asset(source_asset)
                } else {
                    amount.clone()
                }
            })
            .unwrap_or_else(|| amount_for_asset(source_asset));
        let issuer = if source_asset.native() {
            xrp_account()
        } else {
            request.source_account_id
        };
        let source_element = STPathElement::raw(
            STPathElement::TYPE_ACCOUNT
                | if source_asset.native() {
                    0
                } else if matches!(source_asset, Asset::MPTIssue(_)) {
                    STPathElement::TYPE_MPT | STPathElement::TYPE_ISSUER
                } else {
                    STPathElement::TYPE_CURRENCY | STPathElement::TYPE_ISSUER
                },
            request.source_account_id,
            path_asset(source_asset),
            issuer,
        );
        let mut candidates = Discovery {
            ledger: ledger.as_ref(),
            app,
            source_account: request.source_account_id,
            destination_account: request.destination_account_id,
            destination_asset,
            effective_destination,
            source_asset,
            source_element,
            domain: request.domain,
            complete: Vec::new(),
        }
        .discover(search_level);
        if let Some(previous) = previous_paths.get(&source_asset) {
            for path in previous.iter() {
                append_unique(&mut candidates, path.clone());
            }
        }

        let mut ranked = Vec::new();
        for (ordinal, candidate) in candidates.into_iter().enumerate() {
            let Some((quality, liquidity)) = path_liquidity(
                Arc::clone(&ledger),
                request,
                &source_amount,
                &destination_amount,
                &candidate,
            ) else {
                continue;
            };
            ranked.push(RankedPath {
                path: candidate,
                quality,
                liquidity,
                ordinal,
            });
        }
        ranked.sort_by(|a, b| {
            (if request.convert_all {
                std::cmp::Ordering::Equal
            } else {
                a.quality.cmp(&b.quality)
            })
            .then_with(|| b.liquidity.cmp(&a.liquidity))
            .then_with(|| a.path.size().cmp(&b.path.size()))
            .then_with(|| b.ordinal.cmp(&a.ordinal))
        });

        // Match Pathfinder's selection rule: reserve the final slot for a
        // path that can cover the remaining amount. The default path is
        // evaluated first because it consumes none of the four explicit
        // path slots.
        let mut remaining = destination_amount.clone();
        if let Some(default_result) = run_calc(
            Arc::clone(&ledger),
            request,
            &source_amount,
            &remaining,
            &path_set(std::iter::empty::<STPath>()),
            true,
            true,
        ) && is_tes_success(default_result.result)
        {
            remaining -= default_result.actual_amount_out;
        }

        let mut selected = Vec::new();
        let mut covering = None;
        for rank in &ranked {
            let slots_left = MAX_PATHS.saturating_sub(selected.len());
            if slots_left > 1 || (slots_left == 1 && rank.liquidity >= remaining) {
                selected.push(rank.path.clone());
                if remaining.signum() > 0 {
                    remaining -= rank.liquidity.clone();
                }
            } else if covering.is_none() && rank.liquidity >= destination_amount {
                covering = Some(rank.path.clone());
            }
            if selected.len() == MAX_PATHS {
                break;
            }
        }

        let mut selected_set = path_set(selected);
        // rippled persists the ranked selection, not the temporary covering
        // path that may be appended solely for this response's retry.
        path_context.insert(source_asset, selected_set.clone());
        let Some(mut result) = run_calc(
            Arc::clone(&ledger),
            request,
            &source_amount,
            &destination_amount,
            &selected_set,
            true,
            request.convert_all,
        ) else {
            continue;
        };
        if !request.convert_all
            && matches!(result.result, Ter::TER_NO_LINE | Ter::TEC_PATH_PARTIAL)
            && let Some(covering) = covering
        {
            selected_set.push_back(covering);
            let Some(retry) = run_calc(
                Arc::clone(&ledger),
                request,
                &source_amount,
                &destination_amount,
                &selected_set,
                true,
                request.convert_all,
            ) else {
                continue;
            };
            result = retry;
        }
        if result.result != Ter::TES_SUCCESS || result.actual_amount_out.signum() <= 0 {
            continue;
        }

        let mut entry = BTreeMap::from([
            (
                "source_amount".to_owned(),
                result.actual_amount_in.json(JsonOptions::new(0)),
            ),
            (
                "paths_computed".to_owned(),
                selected_set.json(JsonOptions::new(0)),
            ),
        ]);
        if request.convert_all {
            entry.insert(
                "destination_amount".to_owned(),
                result.actual_amount_out.json(JsonOptions::new(0)),
            );
        }
        if legacy {
            entry.insert("paths_canonical".to_owned(), JsonValue::Array(Vec::new()));
        }
        alternatives.push(JsonValue::Object(entry));
    }
    let destination_currencies = if destination_exists {
        let destination_root = ledger
            .read(protocol::account_keylet(Uint160::from_void(
                request.destination_account_id.data(),
            )))
            .ok()
            .flatten();
        let mut currencies = BTreeSet::new();
        if !destination_root
            .as_ref()
            .is_some_and(|root| root.is_flag(protocol::lsfDisallowXRP))
        {
            currencies.insert("XRP".to_owned());
        }
        for entry in owner_entries(&ledger, request.destination_account_id) {
            if entry.get_type() == LedgerEntryType::MPToken {
                let id = entry.get_field_h192(sf("sfMPTokenIssuanceID"));
                if entry.get_field_u64(sf("sfMPTAmount")) == 0 && !mpt_is_maxed(&ledger, id) {
                    currencies.insert(id.to_string());
                }
                continue;
            }
            if entry.get_type() == LedgerEntryType::MPTokenIssuance {
                let id = protocol::make_mpt_id(
                    entry.get_field_u32(sf("sfSequence")),
                    request.destination_account_id,
                );
                if !mpt_is_maxed(&ledger, id) {
                    currencies.insert(id.to_string());
                }
                continue;
            }
            if entry.get_type() != LedgerEntryType::RippleState {
                continue;
            }
            let low = entry.get_field_amount(sf("sfLowLimit"));
            let high = entry.get_field_amount(sf("sfHighLimit"));
            let balance = entry.get_field_amount(sf("sfBalance"));
            let destination_is_low = low.issue().account == request.destination_account_id;
            let limit = if destination_is_low { &low } else { &high };
            let mut viewed_balance = balance.clone();
            if !destination_is_low {
                viewed_balance.negate();
            }
            if viewed_balance < *limit {
                currencies.insert(protocol::currency_to_string(low.issue().currency));
            }
        }
        currencies.into_iter().collect()
    } else {
        vec!["XRP".to_owned()]
    };
    Ok(PathFinderResult {
        alternatives: JsonValue::Array(alternatives),
        ledger_hash: Some(ledger.header().hash.to_string()),
        ledger_index: Some(ledger.header().seq),
        destination_currencies,
        validated: None,
        path_context,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::ApplicationRootOptions;
    use ledger::{LedgerConfig, RawView};

    fn account_root(account: AccountID, drops: i64) -> protocol::STLedgerEntry {
        let mut root = protocol::STLedgerEntry::from_type_and_key(
            LedgerEntryType::AccountRoot,
            protocol::account_keylet(Uint160::from_void(account.data())).key,
        );
        root.set_account_id(sf("sfAccount"), account);
        root.set_field_u32(sf("sfSequence"), 1);
        root.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(drops)),
        );
        root
    }

    #[test]
    fn costed_template_counts_match_rippled() {
        let issuer_a = AccountID::from_array([1; 20]);
        let issuer_b = AccountID::from_array([2; 20]);
        let usd = protocol::currency_from_string("USD");
        let eur = protocol::currency_from_string("EUR");
        let xrp = Asset::Issue(xrp_issue());
        let usd_a = Asset::Issue(Issue::new(usd, issuer_a));
        let usd_b = Asset::Issue(Issue::new(usd, issuer_b));
        let eur_b = Asset::Issue(Issue::new(eur, issuer_b));

        assert_eq!(templates(xrp, usd_a).len(), 7);
        assert_eq!(templates(usd_a, xrp).len(), 6);
        assert_eq!(templates(usd_a, usd_b).len(), 12);
        assert_eq!(templates(usd_a, eur_b).len(), 12);
        assert!(templates(xrp, xrp).is_empty());
    }

    #[test]
    fn direct_xrp_matches_rippled_empty_alternatives() {
        let app =
            ApplicationRoot::with_options(ApplicationRootOptions::default()).expect("application");
        let ledger = Arc::new(
            Ledger::create_genesis(false, &LedgerConfig::default(), []).expect("genesis ledger"),
        );
        let source = AccountID::from_slice(ledger::genesis_master_account_id().data())
            .expect("account width");
        let destination = AccountID::from_array([9; 20]);
        let params = JsonValue::Object(BTreeMap::from([
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(source)),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(destination)),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("20000000".to_owned()),
            ),
        ]));
        let request = app::paths::parse_path_finder_request(&params).expect("request");
        let result =
            find_paths(&app, ledger, &request, 3, true, &BTreeMap::new()).expect("path result");
        let JsonValue::Array(alternatives) = result.alternatives else {
            panic!("alternatives must be an array");
        };
        assert!(alternatives.is_empty());
    }

    #[test]
    fn direct_iou_alternative_uses_sandboxed_ripple_calc_amount() {
        let app =
            ApplicationRoot::with_options(ApplicationRootOptions::default()).expect("application");
        let source = AccountID::from_array([1; 20]);
        let issuer = AccountID::from_array([2; 20]);
        let currency = protocol::currency_from_string("HST");
        let mut ledger = Ledger::from_ledger_seq_and_close_time(10, 100, false);
        ledger
            .raw_insert(Arc::new(account_root(source, 1_000_000_000)))
            .expect("source root");
        ledger
            .raw_insert(Arc::new(account_root(issuer, 1_000_000_000)))
            .expect("issuer root");
        let mut line = protocol::STLedgerEntry::from_type_and_key(
            LedgerEntryType::RippleState,
            protocol::line(source, issuer, currency).key,
        );
        line.set_field_amount(
            sf("sfBalance"),
            STAmount::from_iou_amount(
                protocol::sf_generic(),
                protocol::IOUAmount::from_parts(100, 0).expect("balance"),
                Issue::new(currency, source),
            ),
        );
        line.set_field_amount(
            sf("sfLowLimit"),
            STAmount::from_iou_amount(
                protocol::sf_generic(),
                protocol::IOUAmount::from_parts(1_000, 0).expect("limit"),
                Issue::new(currency, source),
            ),
        );
        line.set_field_amount(
            sf("sfHighLimit"),
            STAmount::from_iou_amount(
                protocol::sf_generic(),
                protocol::IOUAmount::new(),
                Issue::new(currency, issuer),
            ),
        );
        ledger.raw_insert(Arc::new(line)).expect("trust line");
        let ledger = Arc::new(ledger);
        let params = JsonValue::Object(BTreeMap::from([
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(source)),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(issuer)),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    ("currency".to_owned(), JsonValue::String("HST".to_owned())),
                    (
                        "issuer".to_owned(),
                        JsonValue::String(protocol::to_base58(issuer)),
                    ),
                    ("value".to_owned(), JsonValue::String("1".to_owned())),
                ])),
            ),
            (
                "source_currencies".to_owned(),
                JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([(
                    "currency".to_owned(),
                    JsonValue::String("HST".to_owned()),
                )]))]),
            ),
        ]));
        let request = app::paths::parse_path_finder_request(&params).expect("request");
        let result = find_paths(
            &app,
            Arc::clone(&ledger),
            &request,
            3,
            true,
            &BTreeMap::new(),
        )
        .expect("path result");
        let JsonValue::Array(alternatives) = result.alternatives else {
            panic!("alternatives must be array");
        };
        assert_eq!(alternatives.len(), 1);
        let JsonValue::Object(alternative) = &alternatives[0] else {
            panic!("alternative must be object");
        };
        assert_eq!(
            alternative.get("paths_computed"),
            Some(&JsonValue::Array(vec![]))
        );
        let Some(JsonValue::Object(source_amount)) = alternative.get("source_amount") else {
            panic!("source amount must be IOU");
        };
        assert_eq!(
            source_amount.get("value"),
            Some(&JsonValue::String("1".to_owned()))
        );
        assert_eq!(
            source_amount.get("issuer"),
            Some(&JsonValue::String(protocol::to_base58(source)))
        );

        let mut convert_all_params = params.clone();
        {
            let JsonValue::Object(convert_all_object) = &mut convert_all_params else {
                unreachable!("test request is an object");
            };
            let Some(JsonValue::Object(destination_amount)) =
                convert_all_object.get_mut("destination_amount")
            else {
                unreachable!("test destination amount is issued currency");
            };
            destination_amount.insert("value".to_owned(), JsonValue::String("-1".to_owned()));
            convert_all_object.insert(
                "send_max".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    ("currency".to_owned(), JsonValue::String("HST".to_owned())),
                    (
                        "issuer".to_owned(),
                        JsonValue::String(protocol::to_base58(source)),
                    ),
                    ("value".to_owned(), JsonValue::String("10".to_owned())),
                ])),
            );
        }
        let convert_all_request = app::paths::parse_path_finder_request(&convert_all_params)
            .expect("issued -1 convert-all request");
        let convert_all_result = find_paths(
            &app,
            Arc::clone(&ledger),
            &convert_all_request,
            3,
            true,
            &BTreeMap::new(),
        )
        .expect("convert-all path result");
        let JsonValue::Array(convert_all_alternatives) = convert_all_result.alternatives else {
            panic!("convert-all alternatives must be an array");
        };
        assert_eq!(convert_all_alternatives.len(), 1);
        let JsonValue::Object(convert_all_alternative) = &convert_all_alternatives[0] else {
            panic!("convert-all alternative must be an object");
        };
        let Some(JsonValue::Object(delivered)) = convert_all_alternative.get("destination_amount")
        else {
            panic!("convert-all must report the actual destination amount");
        };
        assert_eq!(
            delivered.get("value"),
            Some(&JsonValue::String("10".to_owned()))
        );

        {
            let JsonValue::Object(convert_all_object) = &mut convert_all_params else {
                unreachable!("test request is an object");
            };
            let Some(JsonValue::Object(send_max)) = convert_all_object.get_mut("send_max") else {
                unreachable!("test send max is issued currency");
            };
            send_max.insert("value".to_owned(), JsonValue::String("-1".to_owned()));
        }
        let unlimited_request = app::paths::parse_path_finder_request(&convert_all_params)
            .expect("negative-one send max is unlimited for the matching source issue");
        let unlimited_result =
            find_paths(&app, ledger, &unlimited_request, 3, true, &BTreeMap::new())
                .expect("unlimited convert-all path result");
        let JsonValue::Array(unlimited_alternatives) = unlimited_result.alternatives else {
            panic!("unlimited alternatives must be an array");
        };
        assert_eq!(unlimited_alternatives.len(), 1);
    }

    #[test]
    fn destination_currencies_honor_disallow_xrp() {
        let app =
            ApplicationRoot::with_options(ApplicationRootOptions::default()).expect("application");
        let source = AccountID::from_array([1; 20]);
        let destination = AccountID::from_array([2; 20]);
        let mut ledger = Ledger::from_ledger_seq_and_close_time(10, 100, false);
        ledger
            .raw_insert(Arc::new(account_root(source, 1_000_000_000)))
            .expect("source root");
        let mut destination_root = account_root(destination, 1_000_000_000);
        destination_root.set_field_u32(sf("sfFlags"), protocol::lsfDisallowXRP);
        ledger
            .raw_insert(Arc::new(destination_root))
            .expect("destination root");
        let params = JsonValue::Object(BTreeMap::from([
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(source)),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(destination)),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::Object(BTreeMap::from([
                    ("currency".to_owned(), JsonValue::String("USD".to_owned())),
                    (
                        "issuer".to_owned(),
                        JsonValue::String(protocol::to_base58(destination)),
                    ),
                    ("value".to_owned(), JsonValue::String("1".to_owned())),
                ])),
            ),
        ]));
        let request = app::paths::parse_path_finder_request(&params).expect("request");
        let result = find_paths(&app, Arc::new(ledger), &request, 3, true, &BTreeMap::new())
            .expect("path result");
        assert!(
            !result
                .destination_currencies
                .iter()
                .any(|asset| asset == "XRP")
        );
    }
}
