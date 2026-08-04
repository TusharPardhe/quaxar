# Production replay fixtures — TEST ONLY

This directory is exclusively for deterministic consensus replay fixtures. It
must never be used by the running node, production configuration, or runtime
NodeStore.

A fixture directory contains:

- `manifest.json`: fixture version, fixed parent/child sequences and hashes,
  expected transaction/account/ledger hashes, and bounded resource limits.
- `parent.snapshot`: immutable native NodeStore snapshot containing the full
  canonical parent state.
- `child.json`: canonical child header, transaction blobs, canonical order,
  and expected transaction metadata.

The replay harness is intentionally ignored by default and requires an explicit
`QUAXAR_PRODUCTION_REPLAY_FIXTURE` path. It refuses paths beneath the live node
state directory and applies a hard wall-clock timeout plus a fixture-declared
maximum size before invoking the production acceptance path.
