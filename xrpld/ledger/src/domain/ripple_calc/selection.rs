//! Strand quality calculation and selection helpers for explicit paths.

use protocol::{Amounts, Quality, STAmount};

use crate::{ReadView, ViewError};

use super::RippleCalcInput;
use super::strand::{ExplicitStrand, estimate_direct_strand};

#[derive(Debug, Clone)]
pub(crate) struct RankedStrand {
    pub path_index: usize,
    pub quality: Quality,
    pub hop_count: usize,
}

pub(crate) fn rank_explicit_strands<V: ReadView>(
    view: &V,
    strands: &[ExplicitStrand],
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    input: &RippleCalcInput,
) -> Result<Vec<RankedStrand>, ViewError> {
    let limit_quality = input.limit_quality.then(|| {
        Quality::from_amounts(&Amounts::new(max_source_amount.clone(), dst_amount.clone()))
    });

    let mut ranked = Vec::new();
    for strand in strands {
        let Some(estimate) = estimate_direct_strand(view, strand, dst_amount)? else {
            continue;
        };

        if let Some(limit) = limit_quality
            && estimate.quality < limit
        {
            continue;
        }

        ranked.push(RankedStrand {
            path_index: strand.path_index,
            quality: estimate.quality,
            hop_count: strand.steps.len().max(1),
        });
    }

    ranked.sort_by(|lhs, rhs| {
        rhs.quality
            .cmp(&lhs.quality)
            .then_with(|| lhs.hop_count.cmp(&rhs.hop_count))
            .then_with(|| lhs.path_index.cmp(&rhs.path_index))
    });

    Ok(ranked)
}
