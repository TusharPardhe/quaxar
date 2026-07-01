//!
//! Single-strand execution uses reverse-then-forward:
//! 1. Reverse pass: determine how much input each step needs
//! 2. Forward pass: execute with determined amounts
//!
//! Multi-strand: iterate strands, pick best, accumulate results.

use super::steps::execute_step_fwd;
use super::{FlowResult, Strand};
use crate::ApplyView;
use protocol::{AccountID, Quality, STAmount, Ter};

const MAX_TRIES: usize = 1000;

/// Execute strands to deliver the requested amount.
pub fn execute_strands<V: ApplyView>(
    view: &mut V,
    strands: &[Strand],
    deliver: &STAmount,
    partial_payment: bool,
    send_max: Option<&STAmount>,
    strand_src: &AccountID,
    quality_threshold: Option<Quality>,
) -> FlowResult {
    if strands.is_empty() {
        return FlowResult {
            ter: Ter::TEC_PATH_DRY,
            actual_in: deliver.zeroed(),
            actual_out: deliver.zeroed(),
        };
    }

    let mut remaining_out = deliver.clone();
    let mut remaining_in = send_max.cloned();
    let mut total_in = send_max
        .map(|s| s.zeroed())
        .unwrap_or_else(|| deliver.zeroed());
    let mut total_out = deliver.zeroed();
    let active_strands: Vec<usize> = (0..strands.len()).collect();

    for _cur_try in 0..MAX_TRIES {
        if remaining_out.signum() <= 0 {
            break;
        }
        if let Some(ref ri) = remaining_in
            && ri.signum() <= 0
        {
            break;
        }
        if active_strands.is_empty() {
            break;
        }

        let mut best: Option<(STAmount, STAmount, usize)> = None;

        for (pos, &strand_idx) in active_strands.iter().enumerate() {
            let strand = &strands[strand_idx];
            if strand.is_empty() {
                continue;
            }

            // is below the limit quality (taker's threshold for OfferCreate).
            // NOTE: float-based prefilter disabled — f64 quality comparison diverges
            // from reference Quality/STAmount semantics and causes false negatives.
            // Threshold enforcement happens inside BookStep using XRPL quality types.
            // if let Some(threshold) = quality_threshold {
            //     if !strand_quality_above_threshold(view, strand, threshold) {
            //         continue;
            //     }
            // }

            // Execute strand directly on the view.
            // The caller (ripple_calculate) already wraps us in a FlowSandbox
            // that will be discarded on failure. No need for per-strand sandboxing
            // for single-strand cases (which is the common case on mainnet).
            let max_in = remaining_in.as_ref();

            let result = execute_single_strand(
                view,
                strand,
                max_in,
                &remaining_out,
                strand_src,
                quality_threshold,
            );

            if let Some((in_amt, out_amt)) = result
                && out_amt.signum() > 0
            {
                best = Some((in_amt, out_amt, pos));
                break;
            }
        }

        if let Some((in_amt, out_amt, _)) = best {
            total_in += in_amt.clone();
            total_out += out_amt.clone();
            remaining_out = deliver.clone() - total_out.clone();
            if let Some(sm) = send_max {
                remaining_in = Some(sm.clone() - total_in.clone());
            }
        } else {
            break;
        }
    }

    if total_out.signum() > 0 {
        if total_out < *deliver && !partial_payment {
            return FlowResult {
                ter: Ter::TEC_PATH_PARTIAL,
                actual_in: total_in,
                actual_out: total_out,
            };
        }
        FlowResult {
            ter: Ter::TES_SUCCESS,
            actual_in: total_in,
            actual_out: total_out,
        }
    } else {
        FlowResult {
            ter: Ter::TEC_PATH_DRY,
            actual_in: total_in,
            actual_out: total_out,
        }
    }
}

/// Execute a single strand with proper amount tracking.
///
/// Returns (actual_in, actual_out) or None if strand is dry.
fn execute_single_strand<V: ApplyView>(
    view: &mut V,
    strand: &Strand,
    max_in: Option<&STAmount>,
    max_out: &STAmount,
    strand_src: &AccountID,
    quality_threshold: Option<Quality>,
) -> Option<(STAmount, STAmount)> {
    if strand.is_empty() {
        return None;
    }

    // For forward execution: pass input through each step, tracking actual amounts.
    // The input to the first step is limited by max_in (sendMax).
    // Each subsequent step receives the output of the previous step.
    //
    // The actual_in is what the first step consumed.
    // The actual_out is what the last step produced.

    // Determine input for the first step
    let first_input = if let Some(mi) = max_in {
        mi.clone()
    } else {
        max_out.clone()
    };

    let mut step_input = first_input;
    let mut first_step_consumed = None;

    for (i, step) in strand.iter().enumerate() {
        match execute_step_fwd(
            view,
            step,
            &step_input,
            max_out,
            strand_src,
            quality_threshold,
        ) {
            Ok((consumed_in, produced_out)) => {
                if i == 0 {
                    // Track what the first step actually consumed
                    first_step_consumed = Some(consumed_in);
                }
                if produced_out.signum() <= 0 {
                    return None; // Step produced nothing — strand is dry
                }
                step_input = produced_out;
            }
            Err(_) => {
                return None;
            }
        }
    }

    let actual_out = step_input; // Final output from last step
    if actual_out.signum() <= 0 {
        return None;
    }

    // Limit output to max_out
    let actual_out = if actual_out > *max_out {
        max_out.clone()
    } else {
        actual_out
    };

    // actual_in = what the first step consumed
    let actual_in =
        first_step_consumed.unwrap_or_else(|| max_in.cloned().unwrap_or_else(|| max_out.zeroed()));

    Some((actual_in, actual_out))
}

/// Returns true if the strand's quality is >= threshold (worth executing).
/// For OfferCreate, this prevents crossing strands that would give the taker
/// a worse rate than their own offer quality.
///
/// The strand quality = product of each book step's best offer quality.
/// Direct steps and XRP endpoint steps have quality = 1 (no rate change).
#[allow(dead_code)]
fn strand_quality_above_threshold<V: crate::ReadView>(
    view: &V,
    strand: &super::Strand,
    threshold: f64,
) -> bool {
    use super::StepKind;
    use crate::domain::ripple_calc::book_step::{Book, get_book_best_quality};

    // Compute combined strand quality = product of each book step's best offer quality.
    // quality = output_amount / input_amount for the strand.
    let mut combined_quality: f64 = 1.0;
    let mut has_book_step = false;

    for step in strand.iter() {
        if let StepKind::Book { book_in, book_out } = step {
            has_book_step = true;
            let book = Book {
                r#in: protocol::Asset::Issue(*book_in),
                out: protocol::Asset::Issue(*book_out),
                domain: None,
            };
            // Get the best offer quality in this book (TakerGets/TakerPays = output/input)
            if let Some(q) = get_book_best_quality(view, &book) {
                combined_quality *= q;
            } else {
                return false; // No offers in this book → strand is dry
            }
        }
    }

    if !has_book_step {
        return true; // No book steps → direct transfer, always above threshold
    }

    combined_quality >= threshold
}
