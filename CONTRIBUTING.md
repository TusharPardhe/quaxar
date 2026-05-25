# Contributing to xrpld

Thank you for your interest in contributing to the Rust XRPL node implementation! This guide will help you get started.

## Prerequisites

- **Rust 1.90+** — Install via [rustup](https://rustup.rs/)
- **just** — Command runner: `cargo install just`
- **git** — With conventional commit awareness

## Building

```bash
# Full workspace build
just build

# Release build
cargo build --release

# Single crate
cargo build -p xrpld-app
```

## Testing

```bash
# Run all tests
just test

# Run tests for a specific crate
cargo test -p xrpld-rpc

# Run with nextest (faster, parallel)
cargo nextest run --workspace
```

## Code Style

### Formatting

We use `rustfmt` with the project's `rustfmt.toml`:

```bash
cargo fmt --all
```

### Linting

All clippy warnings are treated as errors in CI:

```bash
cargo clippy --workspace --all-features -- -D warnings
```

### Conventional Commits

All commits must follow the [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>: <description>

[optional body]
```

#### Commit Types

| Type | Description |
|------|-------------|
| `feat` | New feature |
| `fix` | Bug fix |
| `docs` | Documentation only |
| `refactor` | Code refactoring (no behavior change) |
| `test` | Adding or updating tests |
| `perf` | Performance improvement |
| `chore` | Maintenance, dependencies, CI |

#### Examples

```
feat: add account_tx RPC handler
fix: resolve NuDB 48-bit key overflow on large databases
perf: parallelize state map acquisition across 4 threads
docs: add architecture diagram to ARCHITECTURE.md
```

## PR Process

1. **Fork** the repository
2. **Branch** from `main` with a descriptive name: `feat/account-tx-rpc`, `fix/nudb-overflow`
3. **Implement** your changes with tests
4. **Run checks** locally:
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-features -- -D warnings
   cargo test --workspace
   ```
5. **Push** and open a Pull Request
6. **CI must pass** — all lint, test, and build jobs
7. **Review** — at least one approving review required

## Architecture Overview

Understanding which crate owns what helps you find the right place for changes:

| Crate | Responsibility |
|-------|---------------|
| `xrpl/protocol` | Wire types, serialization, protobuf definitions |
| `xrpl/basics` | Utility types, tagged integers, time, config |
| `xrpl/shamap` | SHAMap trie (state and transaction trees) |
| `xrpl/core` | Cryptography, key derivation, signing |
| `xrpl/resource` | Load management and resource tracking |
| `xrpld/app` | Application orchestration, ledger acquisition |
| `xrpld/consensus` | XRPL consensus protocol implementation |
| `xrpld/ledger` | Ledger state, open/closed/validated lifecycle |
| `xrpld/overlay` | P2P networking, peer message handling |
| `xrpld/nodestore` | NuDB storage backend, node object persistence |
| `xrpld/rpc` | JSON-RPC method handlers |
| `xrpld/server` | HTTP/WebSocket server, request routing |
| `xrpld/tx` | Transaction processing and application |
| `xrpld/metrics` | Prometheus metrics collection |
| `xrpld/main` | Binary entry point, CLI, startup |

## Where to Start

Good first contributions:

- **Add missing RPC handlers** — Look at `xrpld/rpc/src/` for the pattern, then implement a missing method
- **Improve test coverage** — Run `cargo llvm-cov` and find uncovered paths
- **Documentation** — Add doc comments to public APIs in any crate
- **CLI commands** — Add new diagnostic commands to the interactive CLI
- **Error messages** — Improve error context with `thiserror` or better `tracing` spans

Look for issues labeled `good first issue` in the issue tracker.

## Getting Help

- Open a GitHub Discussion for questions
- Check [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design context
- Read the learning notes in `docs/learning/` for deep dives into specific subsystems
