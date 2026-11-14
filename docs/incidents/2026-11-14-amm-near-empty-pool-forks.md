# Forks 108321532, 108321579, 108328631: AMM spot-price offers on a near-empty pool

**Date:** 2026-11-14 (forks at 08:56, 09:00 and 16:58 UTC)
**Fixed in:** `xrpld/ledger/src/domain/ripple_calc/book_step.rs`
(`amm_offer_starting_with_gets`, `amm_offer_starting_with_pays`)
**Fixtures:** `mainnet_108328631_batch_amm_zero_output_spot_price_offer.jsonl`,
`mainnet_108321532_batch_amm_drained_pool_spot_price_offer.jsonl`

## What happened

All three are inner Batch self-payments from `rwG5huGC…` (tfPartialPayment):
BTC → AMM `rhzqvs6t` → xOracleGG → book xOracleGG/XRP → XRP. The second book
holds the AMM `rGKeSkN5` (1% fee) and one CLOB offer at 0.001 xOracleGG per
drop. The AMM pool is nearly empty: a few million drops against ~1e-13
xOracleGG at the start of each payment.

With a CLOB tip, rippled sizes the AMM offer with `changeSpotPriceQuality`
→ `getAMMOfferStartWithTakerGets` (XRP out). quaxar's port differed from
rippled at two edges, and both made quaxar fall back to `maxOffer`
(`fixAMMv1_2`), an offer rippled never builds there.

| Fork | Edge | rippled | quaxar before |
|---|---|---|---|
| 108328631 | takerGets rounds down to **0 drops** (2-drop pool) | `swapAssetOut(pool, 0)` is a value; `Quality{(in, 0)}` is rate 0 (best), so the zero offer is returned and `getOffer` drops it as non-positive. The next slice goes to the CLOB. | `amm_swap_asset_out` returns `None` for a zero output → no offer → `maxOffer` sold the last drop from the AMM |
| 108321532, 108321579 | The quadratic proposes the **whole pool**: `c = O² - I·O/q` loses `I·O/q` (≈ 3e-4 against 1.5e13) to cancellation | `swapAssetOut` with `pool.out - assetOut <= 0` returns `toMaxAmount<TIn>`; quality fails, `reduceOffer` retries at 99.99% | `amm_swap_asset_out` returns `None` → `maxOffer` (99%), a different split; the AMM input differed by 2.15e-11 xOracleGG |

```
rippled getAMMOfferStartWithTakerGets
  takerGets = toAmount(nTakerGets, Downward)
  amounts   = {swapAssetOut(pool, takerGets), takerGets}   // never fails
  if Quality{amounts} < target: amounts with reduceOffer(takerGets)
changeSpotPriceQuality → Some(amounts)   // even when takerGets == 0
AMMLiquidity::getOffer → drop if in <= 0 || out <= 0   // no maxOffer fallback
```

IMPORTANT: `maxOffer` is reached only when `changeSpotPriceQuality` returns
nullopt, i.e. the calculation itself failed. A zero or pool-draining
candidate is not a failure in rippled.

## Change

- takerGets (or takerPays) that rounds to zero yields a zero-output offer,
  which `get_amm_offer` already drops as non-positive.
- An output that would empty the pool is priced at `to_max_amount` (rippled
  `toMaxAmount`), so the reduce-and-retry path runs.
- A swap that clips below zero yields zero, as rippled's `swapAssetIn` /
  `swapAssetOut` do.

## Verification

- All three captures replay byte for byte after repairing their read-only
  entries (`repair_domains.py`, `repair_readonly.py`: 69–161 Tickets, 0–1
  Delegate).
- Each fixture fails with only its own fix reverted and passes with both.
