//! Narrow the reference implementation gRPC helper port over the landed Rust ledger surface.

use basics::{base_uint::Uint256, sha_map_hash::SHAMapHash};
use ledger::Ledger;
use shamap::{compare::Delta, traversal::TraversalError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LedgerDiffSpecifier {
    Hash(SHAMapHash),
    Sequence(u32),
    Current,
    Closed,
    Validated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerDiffRequest {
    pub base_ledger: LedgerDiffSpecifier,
    pub desired_ledger: LedgerDiffSpecifier,
    pub include_blobs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerDiffObject {
    pub key: Uint256,
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerDiffResponse {
    pub ledger_objects: Vec<LedgerDiffObject>,
}

#[derive(Debug, Clone, Copy)]
pub enum LedgerDiffResolved<'a> {
    Ledger(&'a Ledger),
    NotValidated,
}

#[derive(Debug)]
pub enum LedgerDiffError {
    BaseLedgerNotFound,
    DesiredLedgerNotFound,
    BaseLedgerNotValidated,
    DesiredLedgerNotValidated,
    TooManyDifferences,
    Traversal(TraversalError),
}

pub trait LedgerDiffSource {
    fn ledger_from_specifier(
        &self,
        specifier: LedgerDiffSpecifier,
    ) -> Option<LedgerDiffResolved<'_>>;
}

fn resolve_ledger<'a, S: LedgerDiffSource>(
    source: &'a S,
    specifier: LedgerDiffSpecifier,
    missing: LedgerDiffError,
    not_validated: LedgerDiffError,
) -> Result<&'a Ledger, LedgerDiffError> {
    match source.ledger_from_specifier(specifier) {
        Some(LedgerDiffResolved::Ledger(ledger)) => Ok(ledger),
        Some(LedgerDiffResolved::NotValidated) => Err(not_validated),
        None => Err(missing),
    }
}

pub fn do_ledger_diff<S: LedgerDiffSource>(
    request: LedgerDiffRequest,
    source: &S,
) -> Result<LedgerDiffResponse, LedgerDiffError> {
    let base = resolve_ledger(
        source,
        request.base_ledger,
        LedgerDiffError::BaseLedgerNotFound,
        LedgerDiffError::BaseLedgerNotValidated,
    )?;
    let desired = resolve_ledger(
        source,
        request.desired_ledger,
        LedgerDiffError::DesiredLedgerNotFound,
        LedgerDiffError::DesiredLedgerNotValidated,
    )?;

    let mut differences = Delta::new();
    let complete = base
        .state_map()
        .compare(
            desired.state_map(),
            &mut differences,
            i32::MAX,
            &mut |_| None,
            &mut |_| None,
        )
        .map_err(LedgerDiffError::Traversal)?;
    if !complete {
        return Err(LedgerDiffError::TooManyDifferences);
    }

    let mut response = LedgerDiffResponse::default();
    for (key, (_in_base, in_desired)) in differences {
        response.ledger_objects.push(LedgerDiffObject {
            key,
            data: if request.include_blobs {
                in_desired.map(|item| item.data().to_vec())
            } else {
                None
            },
        });
    }

    Ok(response)
}
