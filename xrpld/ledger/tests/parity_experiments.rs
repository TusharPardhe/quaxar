// Standalone integration-test entrypoint for the deterministic acquisition
// parity harness. Keeping this outside the in-crate test tree lets the
// harness compile independently of unrelated unit-test modules.
#[path = "acquisition/parity_experiments.rs"]
mod parity_experiments;
