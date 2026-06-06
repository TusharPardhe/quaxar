# Stage 1: Build
FROM rust:1.90-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev librocksdb-dev clang lld cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

ENV CARGO_INCREMENTAL=0
ENV RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld"
ENV ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu

RUN cargo build --release -p xrpld-main && \
    strip target/release/quaxar

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates libssl3 librocksdb8.9 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false xrpld \
    && mkdir -p /var/lib/xrpld /etc/xrpld \
    && chown xrpld:xrpld /var/lib/xrpld

COPY --from=builder /src/target/release/quaxar /usr/local/bin/quaxar
COPY --from=builder /src/xrpld.cfg /etc/xrpld/xrpld.cfg
COPY --from=builder /src/validators.txt /etc/xrpld/validators.txt

USER xrpld
WORKDIR /var/lib/xrpld

EXPOSE 5005 6006 51235

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD /usr/local/bin/quaxar health --rpc-url http://127.0.0.1:5005 || exit 1

ENTRYPOINT ["quaxar"]
CMD ["--conf", "/etc/xrpld/xrpld.cfg"]
