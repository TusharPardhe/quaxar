# rippled develop sync — November 2026

**Date:** 2026-11-11
**Branch:** `sync/rippled-nov-2026`
**Upstream range:** `XRPLF/rippled` `develop` `1f46f75318..58b73715c7` (36 commits,
Oct 26 – Nov 10, after part 3 of the October sync).

IMPORTANT: none of these changes alters ledger results on mainnet. The new
amendment gates nothing yet; the peer-protocol changes drop fields that no
server reads.

## Ported

| rippled | Title | Area | Notes |
|---|---|---|---|
| #8681 `a739f3f07f` | fixCleanup3_6_0 placeholder | amendment registry | Registered `Supported::Yes`, `DefaultNo`. Without it, a majority for fixCleanup3_6_0 would make quaxar amendment-blocked once enabled. |
| #8702 `3e5ce6bcb1` | Stop sending TMTransaction.receiveTimestamp | overlay | The three outbound envelopes (disputed relay, queue relay, requested transactions) send `None`. The inbound value is still parsed and is never acted on, as in rippled. |
| #8701 `775c7d2014` | Remove unused peer-protocol fields | overlay | `TMGetObjectByHash` replies no longer echo the request's `nodeID` as `index`. The other removed fields (`history`, `checkedSignature`, `hops`, `addedTransactions`, `removedTransactions`, `TMEndpoints.nodeID/index`, `fat`) were already never sent. |
| #8669 `daf06ef6c8` | Reject a repeated single-value config section | config | Startup fails with `[<section>] requires exactly one value, not N` for any of the 35 sections rippled reads with `getSingleSection`. Before, quaxar logged a warning and used the default. All shipped configs and the mainnet validator config were checked: none repeats one. |
| #7595 `fccd74f8b3` | Type validation for log_level parameters | RPC | A non-string `severity` or `partition` returns plain `invalidParams`. Before, a non-string partition silently meant `base`. |
| #8645 `ad24db01fa` | Inner Batch txs are never in the open ledger | consensus | `debug_assert!` in `open_ledger_consensus_snapshot`, as rippled's `XRPL_ASSERT` in `onClose`. The behaviour is already tested by `open_batch_applies_only_the_outer_transaction`. |

## Not ported

| rippled | Why |
|---|---|
| #6841 simulate `feeHeavyBurdenRPC`, #7729 noripple_check load type | quaxar's JSON-RPC does not charge per-handler load types (every request is `Reference`). Pre-existing gap; nothing to change in isolation. |
| #7827 null-SLE guards in NoRippleCheck/GatewayBalances | `UNREACHABLE` guards; the Rust handlers already handle a missing SLE as `None`. |
| #8638 `Json::Value::asBool` union member | C++ union read; no behaviour change for valid values, and quaxar's JSON has no union. |
| #8037 Quality `ceilOut` typo | Template name typo only. |
| #8582, #8593 STTx copy/re-parse | C++ performance. |
| #8658 node store reads on stop, #8473 clean exit on bad config | C++ async read queue and exception handling. |
| #8485, #8637, #8655, #8661 | Refactors and assertions with no behaviour change. |
| chore / ci / docs / test-only commits | Nothing to port. |

## Testing

- New tests:
  - `log_level::tests::non_string_severity_or_partition_is_invalid_params` (fails before: a numeric partition returned success).
  - `bootstrap::tests::repeated_single_value_section_is_rejected_like_rippled` (fails before: a repeated section passed validation).
  - `default_amendment_registry_supports_fixes_rippled_supports` now lists fixCleanup3_6_0 (fails before: unknown amendment).
  - Envelope tests assert `receive_timestamp == None`.
- Full workspace suite and all fork-capture replays: see the PR.
