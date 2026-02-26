# syntax=docker/dockerfile:1.7

FROM rust:1.85-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY include ./include
COPY shims ./shims
COPY vendor ./vendor

RUN cargo build --release --locked --bin pyrs

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgcc-s1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /workspace/target/release/pyrs /usr/local/bin/pyrs

ENTRYPOINT ["/usr/local/bin/pyrs"]
CMD ["--version"]
