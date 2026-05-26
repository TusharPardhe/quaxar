default:
    @just --list

# Run all checks (what CI does)
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
    cargo build --release -p xrpld-main

# Install xrpld to ~/.cargo/bin (automatically in PATH)
install:
    cargo install --path xrpld/main

# Uninstall xrpld
uninstall:
    cargo uninstall xrpld-main

# Check compilation
check:
    cargo check --workspace

# Generate docs
doc:
    cargo doc --workspace --no-deps --open

# Audit dependencies
audit:
    cargo deny check

# Run the node
run *ARGS:
    cargo run -p xrpld-main -- {{ARGS}}

# Interactive CLI
cli:
    cargo run -p xrpld-main -- cli

# Clean build artifacts
clean:
    cargo clean
