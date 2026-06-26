# syntax=docker/dockerfile:1.4

# ─── Stage 0: Base ────────────────────────────────────────────────────────────
FROM lukemathwalker/cargo-chef:latest-rust-1.90-slim-bookworm AS chef

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev clang mold cmake perl build-essential \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_INCREMENTAL=0
ENV RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=mold"
ENV OPENSSL_NO_VENDOR=1
WORKDIR /src

# ─── Stage 1: Planner ─────────────────────────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ─── Stage 2: Builder ─────────────────────────────────────────────────────────
FROM chef AS builder

COPY --from=planner /src/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release -p xrpld-main && \
    strip target/release/quaxar

# ─── Stage 3: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false xrpld \
    && mkdir -p /var/lib/rippled/db /etc/xrpld \
    && chown -R xrpld:xrpld /var/lib/rippled /etc/xrpld

COPY --from=builder /src/target/release/quaxar /usr/local/bin/quaxar

USER xrpld
WORKDIR /var/lib/rippled

EXPOSE 5005 6006 51235

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD /usr/local/bin/quaxar health --rpc-url http://127.0.0.1:5005 || exit 1

ENTRYPOINT ["quaxar"]
CMD ["--conf", "/etc/xrpld/xrpld.cfg"]