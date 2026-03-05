# syntax=docker/dockerfile:1.7

FROM rust:bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY include ./include
COPY shims ./shims
COPY vendor ./vendor

RUN cargo build --release --locked --bin pyrs

FROM debian:bookworm-slim AS runtime
ARG CPYTHON_STDLIB_VERSION=3.14.3
ARG CPYTHON_STDLIB_SOURCE_URL=https://www.python.org/ftp/python/3.14.3/Python-3.14.3.tgz
ARG CPYTHON_STDLIB_SOURCE_SHA256=d7fe130d0501ae047ca318fa92aa642603ab6f217901015a1df6ce650d5470cd
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libgcc-s1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /workspace/target/release/pyrs /usr/local/bin/pyrs

RUN mkdir -p "/opt/pyrs/stdlib/${CPYTHON_STDLIB_VERSION}" \
    && curl -fsSL -o "/tmp/Python-${CPYTHON_STDLIB_VERSION}.tgz" "${CPYTHON_STDLIB_SOURCE_URL}" \
    && echo "${CPYTHON_STDLIB_SOURCE_SHA256}  /tmp/Python-${CPYTHON_STDLIB_VERSION}.tgz" | sha256sum -c - \
    && tar -xzf "/tmp/Python-${CPYTHON_STDLIB_VERSION}.tgz" -C /tmp \
    && mv "/tmp/Python-${CPYTHON_STDLIB_VERSION}/Lib" "/opt/pyrs/stdlib/${CPYTHON_STDLIB_VERSION}/Lib" \
    && cp "/tmp/Python-${CPYTHON_STDLIB_VERSION}/LICENSE" "/opt/pyrs/stdlib/${CPYTHON_STDLIB_VERSION}/LICENSE" \
    && rm -rf "/tmp/Python-${CPYTHON_STDLIB_VERSION}" "/tmp/Python-${CPYTHON_STDLIB_VERSION}.tgz"

ENV PYRS_CPYTHON_LIB=/opt/pyrs/stdlib/3.14.3/Lib

ENTRYPOINT ["/usr/local/bin/pyrs"]
CMD ["--version"]
