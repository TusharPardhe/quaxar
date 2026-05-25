//! Explicit-path normalization and strand-construction helpers.
// Kept for compatibility with the broader Pathfinder / PaySteps normalization
// surface; the current Rust owner has not wired this constructor seam into the
// active runtime yet.
#![allow(dead_code)]

use std::array;
use std::collections::BTreeSet;

use protocol::{
    AccountID, Asset, Issue, MPTIssue, PathAsset, STPath, STPathElement, Ter, account_keylet,
    is_consistent, is_xrp_currency, no_account, xrp_account, xrp_issue,
};

use crate::ReadView;

use super::strand::{
    AccountTransferStep, BookStep, NormalizedPath, Strand, StrandError, StrandResult, StrandStep,
};
use super::xrp_endpoint_step::{XrpEndpointContext, XrpEndpointStep};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StrandBuildOptions {
    pub owner_pays_transfer_fee: bool,
    pub offer_crossing: bool,
}

pub fn normalize_path(
    src: AccountID,
    dst: AccountID,
    deliver: Asset,
    send_max: Option<Asset>,
    path: &STPath,
    options: StrandBuildOptions,
) -> StrandResult<NormalizedPath> {
    validate_input_accounts(src, dst)?;
    validate_asset(deliver)?;
    if let Some(send_max) = send_max {
        validate_asset(send_max)?;
        if send_max.issuer() == no_account() {
            return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
        }
    }
    if deliver.issuer() == no_account() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }

    for (index, element) in path.iter().enumerate() {
        validate_path_element(index, path, element)?;
    }

    let initial_asset = initial_asset(send_max, deliver, src);
    let mut normalized = Vec::with_capacity(path.size() + 4);

    normalized.push(source_path_element(src, initial_asset));

    if let Some(send_max) = send_max {
        let path_starts_at_send_max_issuer = path
            .front()
            .is_some_and(|first| first.is_account() && first.account_id() == send_max.issuer());
        if send_max.issuer() != src && !path_starts_at_send_max_issuer {
            normalized.push(account_path_element(send_max.issuer()));
        }
    }

    normalized.extend(path.iter().cloned());

    let last_asset = normalized
        .iter()
        .rev()
        .find(|element| element.has_asset())
        .cloned()
        .ok_or(StrandError::Ter(Ter::TEM_BAD_PATH))?;
    if last_asset.path_asset() != PathAsset::from(deliver)
        || (options.offer_crossing && last_asset.issuer_id() != deliver.issuer())
    {
        normalized.push(offer_path_element(deliver));
    }

    let tail_is_deliver_issuer = normalized
        .last()
        .is_some_and(|element| element.is_account() && element.account_id() == deliver.issuer());
    if !(tail_is_deliver_issuer || dst == deliver.issuer()) {
        normalized.push(account_path_element(deliver.issuer()));
    }

    if !normalized
        .last()
        .is_some_and(|element| element.is_account() && element.account_id() == dst)
    {
        normalized.push(account_path_element(dst));
    }

    if normalized.len() < 2 {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }

    Ok(NormalizedPath {
        source: src,
        destination: dst,
        deliver,
        send_max,
        initial_asset,
        original_path: path.clone(),
        elements: normalized,
        is_default_path: path.empty(),
    })
}

pub fn to_strand<V: ReadView>(
    view: &V,
    src: AccountID,
    dst: AccountID,
    deliver: Asset,
    send_max: Option<Asset>,
    path: &STPath,
    options: StrandBuildOptions,
) -> StrandResult<Strand> {
    let normalized = normalize_path(src, dst, deliver, send_max, path, options)?;
    let mut strand = Strand::new(&normalized);
    let mut cur_asset = normalized.initial_asset;
    let mut seen_direct_assets: [BTreeSet<Asset>; 2] = array::from_fn(|_| BTreeSet::new());
    let mut seen_book_outs = BTreeSet::new();

    for index in 0..normalized.elements.len() - 1 {
        let mut cur = normalized.elements[index].clone();
        let next = normalized.elements[index + 1].clone();

        if cur_asset.holds::<MPTIssue>() && cur.has_currency() {
            cur_asset = Asset::from(Issue::default());
        }

        if let Asset::Issue(issue) = &mut cur_asset {
            if cur.is_account() {
                issue.account = cur.account_id();
            } else if cur.has_issuer() {
                issue.account = cur.issuer_id();
            }
        }

        if cur.has_currency() {
            let mut issue = Issue::new(cur.currency(), cur_asset.issuer());
            if is_xrp_currency(issue.currency) {
                issue.account = xrp_account();
            }
            cur_asset = Asset::from(issue);
        } else if cur.has_mpt() {
            cur_asset = Asset::from(cur.mpt_id());
        }

        if cur.is_account()
            && next.is_account()
            && should_insert_implied_account(&cur, &next, cur_asset)
        {
            let issuer = cur_asset.issuer();
            strand.push(StrandStep::AccountTransfer(AccountTransferStep {
                source: cur.account_id(),
                destination: issuer,
                asset: cur_asset,
            }));
            cur = account_path_element(issuer);
        } else if cur.is_account()
            && next.is_offer()
            && needs_implied_account_before_offer(&cur, cur_asset)
        {
            let issuer = cur_asset.issuer();
            strand.push(StrandStep::AccountTransfer(AccountTransferStep {
                source: cur.account_id(),
                destination: issuer,
                asset: cur_asset,
            }));
            cur = account_path_element(issuer);
        } else if cur.is_offer() && next.is_account() {
            if cur_asset.issuer() != next.account_id() && !next.account_id().is_zero() {
                if cur_asset.native() {
                    if index != normalized.elements.len() - 2 {
                        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
                    }
                    let step = build_xrp_endpoint(
                        view,
                        &strand,
                        next.account_id(),
                        false,
                        true,
                        normalized.deliver,
                        options,
                        &mut seen_direct_assets,
                    )?;
                    strand.push(StrandStep::XrpEndpoint(step));
                } else {
                    strand.push(StrandStep::AccountTransfer(AccountTransferStep {
                        source: cur_asset.issuer(),
                        destination: next.account_id(),
                        asset: cur_asset,
                    }));
                }
            }
            continue;
        }

        if !next.is_offer() && next.has_asset() && next.path_asset() != PathAsset::from(cur_asset) {
            return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
        }

        let step = to_step(
            view,
            &strand,
            &cur,
            &next,
            cur_asset,
            index == normalized.elements.len() - 2,
            normalized.deliver,
            options,
            &mut seen_direct_assets,
            &mut seen_book_outs,
        )?;
        strand.push(step);
    }

    if !check_strand(&strand, src, dst, deliver, send_max) {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }

    Ok(strand)
}

fn validate_input_accounts(src: AccountID, dst: AccountID) -> StrandResult<()> {
    if src.is_zero() || dst.is_zero() || src == no_account() || dst == no_account() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    Ok(())
}

fn validate_asset(asset: Asset) -> StrandResult<()> {
    if asset.visit(
        |issue| !is_consistent(*issue),
        |issue| issue.issuer().is_zero(),
    ) {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    Ok(())
}

fn validate_path_element(index: usize, path: &STPath, element: &STPathElement) -> StrandResult<()> {
    let node_type = element.node_type();
    if (node_type & !STPathElement::TYPE_ALL) != 0 || node_type == STPathElement::TYPE_NONE {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }

    let has_account = (node_type & STPathElement::TYPE_ACCOUNT) != 0;
    let has_issuer = (node_type & STPathElement::TYPE_ISSUER) != 0;
    let has_currency = (node_type & STPathElement::TYPE_CURRENCY) != 0;
    let has_mpt = (node_type & STPathElement::TYPE_MPT) != 0;
    let has_asset = (node_type & STPathElement::TYPE_ASSET) != 0;

    if has_account && (has_issuer || has_currency) {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if has_issuer && element.issuer_id().is_zero() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if has_account && element.account_id().is_zero() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if has_currency
        && has_issuer
        && (is_xrp_currency(element.currency()) != element.issuer_id().is_zero())
    {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if has_issuer && element.issuer_id() == no_account() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if has_account && element.account_id() == no_account() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if has_mpt && (has_currency || has_account) {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if has_mpt && has_issuer && element.issuer_id() != Asset::from(element.mpt_id()).issuer() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if index > 0 && path[index - 1].has_mpt() && (has_account || (has_issuer && !has_asset)) {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }

    Ok(())
}

fn initial_asset(send_max: Option<Asset>, deliver: Asset, src: AccountID) -> Asset {
    let asset = send_max.unwrap_or(deliver);
    asset.visit(
        |issue| {
            if issue.native() {
                Asset::from(xrp_issue())
            } else {
                Asset::from(Issue::new(issue.currency, src))
            }
        },
        |issue| Asset::from(*issue),
    )
}

fn source_path_element(src: AccountID, asset: Asset) -> STPathElement {
    STPathElement::from_optionals(
        Some(src),
        Some(PathAsset::from(asset)),
        Some(asset.issuer()),
    )
}

fn account_path_element(account: AccountID) -> STPathElement {
    STPathElement::from_optionals(Some(account), None, None)
}

fn offer_path_element(asset: Asset) -> STPathElement {
    STPathElement::from_optionals(None, Some(PathAsset::from(asset)), Some(asset.issuer()))
}

fn should_insert_implied_account(
    cur: &STPathElement,
    next: &STPathElement,
    cur_asset: Asset,
) -> bool {
    if let Asset::Issue(issue) = cur_asset {
        !issue.native() && issue.account != cur.account_id() && issue.account != next.account_id()
    } else {
        false
    }
}

fn needs_implied_account_before_offer(cur: &STPathElement, cur_asset: Asset) -> bool {
    if let Asset::Issue(issue) = cur_asset {
        issue.account != cur.account_id()
    } else {
        false
    }
}

fn to_step<V: ReadView>(
    view: &V,
    strand: &Strand,
    current: &STPathElement,
    next: &STPathElement,
    cur_asset: Asset,
    is_last: bool,
    deliver: Asset,
    options: StrandBuildOptions,
    seen_direct_assets: &mut [BTreeSet<Asset>; 2],
    seen_book_outs: &mut BTreeSet<Asset>,
) -> StrandResult<StrandStep> {
    if strand.steps.is_empty()
        && current.is_account()
        && current.has_currency()
        && current.path_asset().is_xrp()
    {
        let step = build_xrp_endpoint(
            view,
            strand,
            current.account_id(),
            true,
            false,
            deliver,
            options,
            seen_direct_assets,
        )?;
        return Ok(StrandStep::XrpEndpoint(step));
    }

    if is_last && is_internal_xrp_account(current) && next.is_account() {
        let step = build_xrp_endpoint(
            view,
            strand,
            next.account_id(),
            false,
            true,
            deliver,
            options,
            seen_direct_assets,
        )?;
        return Ok(StrandStep::XrpEndpoint(step));
    }

    if current.is_account() && next.is_account() {
        return build_direct_step(
            view,
            strand,
            current.account_id(),
            next.account_id(),
            cur_asset,
            is_last,
            seen_direct_assets,
            seen_book_outs,
        );
    }

    if current.is_offer() && next.is_account() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }

    let out_token = if next.has_asset() {
        next.path_asset()
    } else {
        PathAsset::from(cur_asset)
    };
    let out_issuer = if next.has_issuer() {
        next.issuer_id()
    } else {
        cur_asset.issuer()
    };

    if cur_asset.native() && out_token.is_xrp() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if !next.is_offer() {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }

    let out_asset = asset_from_token(out_token, out_issuer);
    build_book_step(
        view,
        cur_asset,
        out_asset,
        seen_direct_assets,
        seen_book_outs,
    )
}

fn build_xrp_endpoint<V: ReadView>(
    view: &V,
    strand: &Strand,
    account: AccountID,
    is_first: bool,
    is_last: bool,
    deliver: Asset,
    options: StrandBuildOptions,
    seen_direct_assets: &mut [BTreeSet<Asset>; 2],
) -> StrandResult<XrpEndpointStep> {
    let step = XrpEndpointStep::new(
        view,
        account,
        XrpEndpointContext {
            is_first,
            is_last,
            offer_crossing: options.offer_crossing,
            strand_deliver: deliver,
        },
    )?;

    let issue_index = if is_last { 0 } else { 1 };
    if !seen_direct_assets[issue_index].insert(step.loop_asset()) {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH_LOOP));
    }

    let _ = strand;
    Ok(step)
}

fn build_direct_step<V: ReadView>(
    view: &V,
    strand: &Strand,
    source: AccountID,
    destination: AccountID,
    asset: Asset,
    is_last: bool,
    seen_direct_assets: &mut [BTreeSet<Asset>; 2],
    seen_book_outs: &BTreeSet<Asset>,
) -> StrandResult<StrandStep> {
    if source.is_zero() || destination.is_zero() || source == destination {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if !view.exists(account_keylet(account_to_uint160(source)))? {
        return Err(StrandError::Ter(Ter::TER_NO_ACCOUNT));
    }

    let src_asset = retarget_asset(asset, source);
    let dst_asset = retarget_asset(asset, destination);
    if seen_book_outs.contains(&src_asset)
        && strand
            .steps
            .last()
            .and_then(|step| step.book())
            .is_none_or(|book| book.output != src_asset)
    {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH_LOOP));
    }
    if !seen_direct_assets[0].insert(src_asset) || !seen_direct_assets[1].insert(dst_asset) {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH_LOOP));
    }

    if let Asset::Issue(issue) = asset
        && !issue.native()
        && !(strand.steps.is_empty() && is_last)
        && destination_globally_frozen(view, destination)?
    {
        return Err(StrandError::Ter(Ter::TER_NO_LINE));
    }

    Ok(StrandStep::AccountTransfer(AccountTransferStep {
        source,
        destination,
        asset,
    }))
}

fn build_book_step<V: ReadView>(
    view: &V,
    input: Asset,
    output: Asset,
    seen_direct_assets: &mut [BTreeSet<Asset>; 2],
    seen_book_outs: &mut BTreeSet<Asset>,
) -> StrandResult<StrandStep> {
    if input == output {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if input.visit(|issue| !is_consistent(*issue), |_| false)
        || output.visit(|issue| !is_consistent(*issue), |_| false)
    {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH));
    }
    if !seen_book_outs.insert(output)
        || seen_direct_assets[0].contains(&output)
        || seen_direct_assets[1].contains(&output)
    {
        return Err(StrandError::Ter(Ter::TEM_BAD_PATH_LOOP));
    }
    if !issuer_exists(view, input.issuer())? || !issuer_exists(view, output.issuer())? {
        return Err(StrandError::Ter(Ter::TEC_NO_ISSUER));
    }

    Ok(StrandStep::Book(BookStep { input, output }))
}

fn check_strand(
    strand: &Strand,
    src: AccountID,
    dst: AccountID,
    deliver: Asset,
    send_max: Option<Asset>,
) -> bool {
    let mut cur_account = src;
    let mut cur_asset = initial_asset(send_max, deliver, src);

    for step in &strand.steps {
        let (step_src, step_dst) = step.direct_accounts();
        if step_src != cur_account {
            return false;
        }

        if let Some(book) = step.book() {
            if cur_asset != book.input {
                return false;
            }
            cur_asset = book.output;
        } else if let Asset::Issue(issue) = &mut cur_asset {
            issue.account = step_dst;
        }

        cur_account = step_dst;
    }

    cur_account == dst
        && cur_asset.token() == deliver.token()
        && (cur_asset.issuer() == deliver.issuer() || cur_asset.issuer() == dst)
}

fn is_internal_xrp_account(element: &STPathElement) -> bool {
    element.node_type() == STPathElement::TYPE_ACCOUNT && element.account_id().is_zero()
}

fn asset_from_token(token: PathAsset, issuer: AccountID) -> Asset {
    token.visit(
        |currency| {
            if is_xrp_currency(currency) {
                Asset::from(xrp_issue())
            } else {
                Asset::from(Issue::new(currency, issuer))
            }
        },
        Asset::from,
    )
}

fn retarget_asset(asset: Asset, account: AccountID) -> Asset {
    asset.visit(
        |issue| {
            if issue.native() {
                Asset::from(xrp_issue())
            } else {
                Asset::from(Issue::new(issue.currency, account))
            }
        },
        |issue| Asset::from(*issue),
    )
}

fn issuer_exists<V: ReadView>(view: &V, issuer: AccountID) -> Result<bool, crate::ViewError> {
    if issuer.is_zero() {
        return Ok(true);
    }
    view.exists(account_keylet(account_to_uint160(issuer)))
}

fn destination_globally_frozen<V: ReadView>(
    view: &V,
    destination: AccountID,
) -> Result<bool, crate::ViewError> {
    if destination.is_zero() {
        return Ok(false);
    }
    let Some(account_root) = view.read(account_keylet(account_to_uint160(destination)))? else {
        return Ok(false);
    };
    Ok(account_root.is_flag(protocol::lsfGlobalFreeze))
}

fn account_to_uint160(account: AccountID) -> basics::base_uint::Uint160 {
    basics::base_uint::Uint160::from_slice(account.data())
        .expect("account width should match Uint160")
}
