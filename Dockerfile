# Stage 1: Build
FROM rust:1.90-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev clang lld cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

ENV CARGO_INCREMENTAL=0
ENV RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld"

RUN cargo build --release -p xrpld-main && \
    strip target/release/xrpld

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false xrpld \
    && mkdir -p /var/lib/xrpld /etc/xrpld \
    && chown xrpld:xrpld /var/lib/xrpld

COPY --from=builder /src/target/release/xrpld /usr/local/bin/xrpld
COPY --from=builder /src/xrpld.cfg /etc/xrpld/xrpld.cfg

USER xrpld
WORKDIR /var/lib/xrpld

EXPOSE 5055 6066 51235

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD /usr/local/bin/xrpld health --rpc-url http://127.0.0.1:5055 || exit 1

ENTRYPOINT ["xrpld"]
CMD ["--conf", "/etc/xrpld/xrpld.cfg"]
