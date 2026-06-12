# Stage 1: Dependency cache (only rebuilds when Cargo.toml/Cargo.lock change)
FROM debian:bookworm-slim AS deps

RUN apt-get update && apt-get install -y \
    curl build-essential pkg-config libssl-dev clang lld cmake perl \
    && rm -rf /var/lib/apt/lists/* \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.90.0

ENV PATH="/root/.cargo/bin:${PATH}"
ENV CARGO_INCREMENTAL=0
ENV RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld"

WORKDIR /src

# Copy only manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY xrpl/basics/Cargo.toml xrpl/basics/Cargo.toml
COPY xrpl/protocol/Cargo.toml xrpl/protocol/Cargo.toml
COPY xrpl/shamap/Cargo.toml xrpl/shamap/Cargo.toml
COPY xrpld/app/Cargo.toml xrpld/app/Cargo.toml
COPY xrpld/consensus/Cargo.toml xrpld/consensus/Cargo.toml
COPY xrpld/ledger/Cargo.toml xrpld/ledger/Cargo.toml
COPY xrpld/main/Cargo.toml xrpld/main/Cargo.toml
COPY xrpld/overlay/Cargo.toml xrpld/overlay/Cargo.toml

# Create dummy lib.rs files so cargo can resolve the workspace
RUN find . -name "Cargo.toml" -path "*/xrpl/*" -exec sh -c 'mkdir -p $(dirname {})/src && touch $(dirname {})/src/lib.rs' \; && \
    find . -name "Cargo.toml" -path "*/xrpld/*" ! -path "*/main/*" -exec sh -c 'mkdir -p $(dirname {})/src && touch $(dirname {})/src/lib.rs' \; && \
    mkdir -p xrpld/main/src && echo 'fn main() {}' > xrpld/main/src/main.rs

# Build dependencies only (cached unless Cargo.toml/lock changes)
RUN cargo build --release -p xrpld-main 2>/dev/null || true

# Stage 2: Build with source (fast - only recompiles our crates)
FROM deps AS builder

# Copy actual source (invalidates cache only for source changes)
COPY . .

# Touch all lib.rs/main.rs to ensure they rebuild (not the dummy ones)
RUN find . -name "*.rs" -path "*/src/*" -newer Cargo.lock -exec touch {} +

RUN cargo build --release -p xrpld-main && \
    strip target/release/quaxar

# Stage 3: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false xrpld \
    && mkdir -p /var/lib/xrpld /etc/xrpld \
    && chown xrpld:xrpld /var/lib/xrpld

COPY --from=builder /src/target/release/quaxar /usr/local/bin/quaxar

USER xrpld
WORKDIR /var/lib/xrpld

EXPOSE 5005 6006 51235

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD /usr/local/bin/quaxar health --rpc-url http://127.0.0.1:5005 || exit 1

ENTRYPOINT ["quaxar"]
CMD ["--conf", "/etc/xrpld/xrpld.cfg"]
