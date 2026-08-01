# Overlay slot admission parity

## ✅ EQUIVALENT

`rippled` makes PeerFinder `Slot` objects the authority for connection admission and lifecycle transitions (`Accept` through `Connected`/`Active` to `Closing`).

Quaxar intentionally keeps reduce-relay selection in `overlay/src/peer/slot.rs` and implements connection-capacity admission in `OverlayImpl` with directional active-peer counts and `Setup::peer_limits()`. Activation enforces inbound and outbound budgets, honors fixed, reserved, and cluster peer exceptions, and `on_peer_deactivate` removes the peer before capacity is reused.

The architecture differs, but the observable admission and release semantics are equivalent: a peer is admitted only when its directional capacity is available, and capacity is released on disconnection. A PeerFinder-slot structural refactor is therefore not intended.