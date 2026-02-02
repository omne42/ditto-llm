# syntax=docker/dockerfile:1.7

FROM rust:1.85-slim-bookworm AS builder

WORKDIR /workspace

# ditto-llm optionally depends on a local `../safe-fs-tools` crate (feature `agent`).
# To keep the container build self-contained, we provide a tiny placeholder crate at
# that path. It is NOT used unless `agent` is enabled.
RUN mkdir -p /workspace/safe-fs-tools/src && \
    printf '%s\n' \
      '[package]' \
      'name = "safe-fs-tools"' \
      'version = "0.0.0"' \
      'edition = "2024"' \
      '' \
      '[lib]' \
      'path = "src/lib.rs"' \
      > /workspace/safe-fs-tools/Cargo.toml && \
    printf '%s\n' 'pub fn _placeholder() {}' > /workspace/safe-fs-tools/src/lib.rs

COPY . /workspace/ditto-llm
WORKDIR /workspace/ditto-llm

ARG DITTO_FEATURES="all gateway gateway-config-yaml gateway-translation gateway-proxy-cache gateway-routing-advanced gateway-metrics-prometheus gateway-costing gateway-tokenizer gateway-store-sqlite gateway-store-redis gateway-otel"

RUN cargo build --release --bin ditto-gateway --features "${DITTO_FEATURES}"


FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 10001 --shell /usr/sbin/nologin ditto
USER ditto
WORKDIR /data

COPY --from=builder /workspace/ditto-llm/target/release/ditto-gateway /usr/local/bin/ditto-gateway

EXPOSE 8080
ENTRYPOINT ["ditto-gateway"]
