//! Reverse-then-forward strand execution.
//!
//! This follows rippled's `StrandFlow.h`: evaluate a candidate in reverse to
//! discover its asset-correct required input and first limiting step, discard
//! that probe sandbox, then replay the limiting suffix/prefix in a fresh
//! sandbox.  Only a successful chosen strand is applied to its parent view.

use super::steps::{FlowStep, StepAmount, StepAmounts, StepContext};
use super::{FlowResult, Strand};
use crate::{ApplyView, FlowSandbox};
use protocol::{AccountID, Quality, STAmount, Ter};

const MAX_TRIES: usize = 1000;

#[derive(Debug, Clone)]
struct ReverseProbe {
    cache: Vec<Option<StepAmounts>>,
    limiting_step: Option<usize>,
    // `true` is the special `maxIn` limiter at step zero.  The final run must
    // call fwd on step zero, rather than comparing it to a differently issued
    // deliver amount.
    limited_by_max_in: bool,
}

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
            actual_in: send_max
                .map(STAmount::zeroed)
                .unwrap_or_else(|| deliver.zeroed()),
            actual_out: deliver.zeroed(),
        };
    }

    // The aggregate flow is also transactional. Individual chosen strands
    // apply only to this sandbox; an overall PATH_PARTIAL/PATH_DRY drops all
    // of them exactly as rippled's finishFlow drops its returned sandbox.
    let mut aggregate = FlowSandbox::new(view);
    let mut remaining_out = deliver.clone();
    let mut remaining_in = send_max.cloned();
    let mut total_in = send_max
        .map(STAmount::zeroed)
        .unwrap_or_else(|| deliver.zeroed());
    let mut total_out = deliver.zeroed();
    let mut active: Vec<bool> = vec![true; strands.len()];

    for _ in 0..MAX_TRIES {
        if remaining_out.signum() <= 0 || remaining_in.as_ref().is_some_and(|v| v.signum() <= 0) {
            break;
        }

        let mut applied = false;
        for (strand_index, strand) in strands.iter().enumerate() {
            if !active[strand_index] || strand.is_empty() {
                active[strand_index] = false;
                continue;
            }

            // Every candidate, including a dry probe, owns a child sandbox.
            // No mutation reaches `view` unless this candidate produced a
            // valid amount and was selected below.
            let result = execute_single_strand(
                &mut aggregate,
                strand,
                remaining_in.as_ref(),
                &remaining_out,
                strand_src,
                quality_threshold,
            );

            let Some((amount_in, amount_out, inactive)) = result else {
                active[strand_index] = false;
                continue;
            };
            if amount_out.signum() <= 0 {
                active[strand_index] = false;
                continue;
            }

            total_in += amount_in;
            total_out += amount_out;
            remaining_out = deliver.clone() - total_out.clone();
            if let Some(send_max) = send_max {
                remaining_in = Some(send_max.clone() - total_in.clone());
            }
            if inactive {
                active[strand_index] = false;
            }
            applied = true;
            break;
        }

        if !applied {
            break;
        }
    }

    let result = if total_out.signum() <= 0 {
        FlowResult {
            ter: Ter::TEC_PATH_DRY,
            actual_in: total_in,
            actual_out: total_out,
        }
    } else if total_out < *deliver && !partial_payment {
        FlowResult {
            ter: Ter::TEC_PATH_PARTIAL,
            actual_in: total_in,
            actual_out: total_out,
        }
    } else {
        FlowResult {
            ter: Ter::TES_SUCCESS,
            actual_in: total_in,
            actual_out: total_out,
        }
    };

    if result.ter == Ter::TES_SUCCESS && aggregate.apply().is_err() {
        return FlowResult {
            ter: Ter::TEF_INTERNAL,
            actual_in: deliver.zeroed(),
            actual_out: deliver.zeroed(),
        };
    }
    result
}

/// Run one strand against two nested sandboxes: a discarded reverse probe and
/// a fresh final sandbox.  This reset is the Rust equivalent of rippled's
/// `sb.emplace(&baseView)` when a limiting step is discovered.
fn execute_single_strand<V: ApplyView>(
    parent: &mut V,
    strand: &Strand,
    max_in: Option<&STAmount>,
    requested_out: &STAmount,
    strand_src: &AccountID,
    quality_threshold: Option<Quality>,
) -> Option<(STAmount, STAmount, bool)> {
    let context = StepContext {
        strand_src,
        quality_threshold,
    };

    let probe = {
        let mut dry_sandbox = FlowSandbox::new(parent);
        reverse_probe(&mut dry_sandbox, strand, max_in, requested_out, context)?
    };
    // `dry_sandbox` was deliberately dropped without `apply`.

    let mut final_sandbox = FlowSandbox::new(parent);
    let final_cache = replay_probe(
        &mut final_sandbox,
        strand,
        max_in,
        requested_out,
        context,
        &probe,
    )?;

    let first = final_cache.first()?.as_ref()?;
    let last = final_cache.last()?.as_ref()?;
    if first.input.is_zero() || last.output.is_zero() {
        return None;
    }

    // A successful strand alone is allowed to affect the accumulated parent.
    // This parent can itself be the transaction-level flow sandbox.
    final_sandbox.apply().ok()?;
    let inactive = false; // StepKind currently has no offer-budget inactive state.
    Some((
        first.input.amount().clone(),
        last.output.amount().clone(),
        inactive,
    ))
}

/// Reverse through the strand to discover exact per-step required inputs.
/// Any liquidity or max-input limit records the first limiting step.
fn reverse_probe<V: ApplyView>(
    sandbox: &mut V,
    strand: &Strand,
    max_in: Option<&STAmount>,
    requested_out: &STAmount,
    context: StepContext<'_>,
) -> Option<ReverseProbe> {
    let mut cache = vec![None; strand.len()];
    let mut step_out = StepAmount::new(requested_out.clone());
    let mut limiting_step = None;
    let mut limited_by_max_in = false;

    for index in (0..strand.len()).rev() {
        let amounts = strand[index].rev(sandbox, &step_out, context).ok()?;
        if amounts.output.is_zero() {
            return None;
        }

        let output_limited = !amounts.output.equivalent(&step_out);

        // rippled tests the first step's SendMax before treating that same
        // step's short output as a liquidity limit. The input is tagged, so a
        // malformed strand cannot turn this into a USD-vs-XRP comparison.
        if index == 0 {
            if let Some(max_in) = max_in {
                match amounts.input.greater_than(max_in) {
                    Some(true) => {
                        limiting_step = Some(0);
                        limited_by_max_in = true;
                    }
                    Some(false) if output_limited && limiting_step.is_none() => {
                        limiting_step = Some(index);
                    }
                    Some(false) => {}
                    None => return None,
                }
            } else if output_limited && limiting_step.is_none() {
                limiting_step = Some(index);
            }
        } else if output_limited && limiting_step.is_none() {
            limiting_step = Some(index);
        }

        cache[index] = Some(amounts.clone());
        step_out = amounts.input;
    }

    Some(ReverseProbe {
        cache,
        limiting_step,
        limited_by_max_in,
    })
}

/// Replay the amount plan in the fresh sandbox.  This is the exact shape of
/// rippled's post-reset execution: re-run the limiting step, reverse before
/// it, then forward after it.  With no limiter, a complete reverse pass is
/// replayed and its sandbox becomes the result.
fn replay_probe<V: ApplyView>(
    sandbox: &mut V,
    strand: &Strand,
    max_in: Option<&STAmount>,
    requested_out: &STAmount,
    context: StepContext<'_>,
    probe: &ReverseProbe,
) -> Option<Vec<Option<StepAmounts>>> {
    let mut cache = vec![None; strand.len()];

    let limiting = probe.limiting_step.unwrap_or(strand.len());
    if probe.limited_by_max_in {
        let max_in = max_in?;
        let expected = probe.cache[0].as_ref()?;
        let actual = strand[0]
            .fwd(sandbox, &StepAmount::new(max_in.clone()), expected, context)
            .ok()?;
        if actual.output.is_zero() || !actual.input.equivalent(&StepAmount::new(max_in.clone())) {
            return None;
        }
        cache[0] = Some(actual);
    } else {
        // If the initial reverse pass found a liquidity limit, its reduced
        // output is what must be re-executed after resetting the sandbox.
        let mut step_out = if limiting < strand.len() {
            probe.cache[limiting].as_ref()?.output.clone()
        } else {
            StepAmount::new(requested_out.clone())
        };
        let reverse_start = if limiting < strand.len() {
            limiting
        } else {
            strand.len() - 1
        };

        for index in (0..=reverse_start).rev() {
            let actual = strand[index].rev(sandbox, &step_out, context).ok()?;
            if actual.output.is_zero() || !actual.output.equivalent(&step_out) {
                return None;
            }
            step_out = actual.input.clone();
            cache[index] = Some(actual);
        }
    }

    let forward_start = if probe.limited_by_max_in {
        1
    } else {
        limiting + 1
    };
    let mut step_in = if probe.limited_by_max_in {
        cache[0].as_ref()?.output.clone()
    } else if limiting < strand.len() {
        cache[limiting].as_ref()?.output.clone()
    } else {
        // No limiting step means the reversed execution already processed all
        // steps.  There is no forward suffix to replay.
        return Some(cache);
    };

    for index in forward_start..strand.len() {
        let expected = probe.cache[index].as_ref()?;
        let actual = strand[index]
            .fwd(sandbox, &step_in, expected, context)
            .ok()?;
        if actual.output.is_zero() || !actual.input.equivalent(&step_in) {
            return None;
        }
        step_in = actual.output.clone();
        cache[index] = Some(actual);
    }

    Some(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{Issue, XRPAmount, get_field_by_symbol};

    fn sf(name: &str) -> &'static protocol::SField {
        get_field_by_symbol(name)
    }

    #[test]
    fn reverse_send_max_uses_first_step_asset_not_deliver_asset() {
        let issuer = AccountID::from_array([7; 20]);
        let usd = protocol::currency_from_string("USD");
        let required_usd = StepAmount::new(STAmount::from_iou_amount(
            sf("sfAmount"),
            protocol::IOUAmount::from_parts(12, 0).expect("valid iou"),
            Issue::new(usd, issuer),
        ));
        let send_max_usd = STAmount::from_iou_amount(
            sf("sfAmount"),
            protocol::IOUAmount::from_parts(10, 0).expect("valid iou"),
            Issue::new(usd, issuer),
        );
        let deliver_xrp = STAmount::from_xrp_amount(XRPAmount::from_drops(1));

        assert_eq!(required_usd.greater_than(&send_max_usd), Some(true));
        // A USD requirement cannot be numerically compared with an XRP
        // delivery limit; the tagged amount rejects that operation.
        assert_eq!(required_usd.greater_than(&deliver_xrp), None);
    }
}
