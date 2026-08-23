# Canonical OfferCreate 3E8EFC65 fixture

This fixture preserves the public XRPL JSON-RPC evidence for canonical transaction `3E8EFC654307CE6940C803A1CD22858258A749EDFCA36C95D73FA72BDC730D0B` in validated ledger `106132761` (transaction index `73`). It was downloaded from `https://s1.ripple.com:51234/` at the time recorded in `manifest.json`.

## Canonical outcome

The transaction is an `OfferCreate` from `rw7nJtEN2YuLt57AfAKdfyFMMLHhTMJKdu`: it offers BRRL `255395` for RLUSD `50000`. The BRRL issuer has `TickSize=5`. Canonical metadata reports `tesSUCCESS`, creates offer `3441193612157BB5AAB7675DBE11B61EB73E0014ED7BD34F562565644E135C02`, and rounds `TakerGets` to BRRL `255388.7016038411` at quality `5406f49bd58a9000`.

The parent ledger, creator and issuer account roots, both creator trust lines, and both book directions are retained here. They establish that the creator had BRRL funds, could accept RLUSD, neither asset was globally frozen, and the reverse-book offer was worse than this placement rather than a crossing fill.

## Root cause and source authority

Quaxar recorded `preflight=tesSUCCESS preclaim=tesSUCCESS apply=tefEXCEPTION`. The bug was confined to OfferCreate tick-size application: its local `quality_to_rate_amount` constructed the fractional arithmetic rate with `Issue::default()`. That default is the native XRP issue, so native canonicalization discarded the fractional IOU/IOU rate and the buy-side division failed.

Upstream authority is `rippled/src/libxrpl/tx/transactors/dex/OfferCreate.cpp`, `OfferCreate::applyGuts`, lines 679-703 in this checkout. It computes `Quality{saTakerGets, saTakerPays}.round(uTickSize).rate()` and passes that **no-issue arithmetic rate** to `divide`. Quaxar now uses `protocol::no_issue()` for both zero and nonzero rate construction. The source regression in `xrpld/app/tests/integration/offer_crossing.rs` verifies the exact rounded result and successful offer placement.
