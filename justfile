default:
    @just --list

# Run the local quality suite
ci: fmt clippy test doc

# Format code
fmt:
    cargo fmt --all

# Lint
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run tests
test:
    cargo test --workspace

# Build release binary
build:
    cargo build --release -p quaxar-main

# Install Quaxar to ~/.cargo/bin (automatically in PATH)
install:
    cargo install --path xrpld/main --locked

# Uninstall Quaxar
uninstall:
    cargo uninstall quaxar-main

# Check compilation
check:
    cargo check --workspace

# Generate docs
doc:
    cargo doc --workspace --no-deps

# Generate and open docs locally
doc-open:
    cargo doc --workspace --no-deps --open

# Audit dependencies
audit:
    cargo deny check

# Run the node
run *ARGS:
    cargo run -p quaxar-main -- {{ARGS}}

# Interactive CLI
cli:
    cargo run -p quaxar-main -- cli

# Clean build artifacts
clean:
    cargo clean
