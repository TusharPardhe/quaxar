# Stage 1: Build (same base as runtime to avoid glibc mismatch)
FROM debian:bookworm-slim AS builder

RUN apt-get update && apt-get install -y \
    curl build-essential pkg-config libssl-dev clang lld cmake perl \
    && rm -rf /var/lib/apt/lists/* \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.90.0

ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /src
COPY . .

ENV CARGO_INCREMENTAL=0
ENV RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld"

RUN cargo build --release -p xrpld-main && \
    strip target/release/quaxar

# Stage 2: Runtime
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
