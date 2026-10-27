# syntax=docker/dockerfile:1
# Build on a workstation or CI runner, then transfer the finished image to the VPS.
FROM rust:1.98.1-bookworm AS toolchain
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config libssl-dev python3 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY rust-toolchain.toml ./
RUN rustup target add wasm32-unknown-unknown

FROM toolchain AS frontend-tools
RUN --mount=type=cache,id=brunch-cargo-registry,target=/usr/local/cargo/registry \
    cargo install trunk --locked --version 0.21.14 \
    && cargo install wasm-bindgen-cli --locked --version 0.2.129

FROM frontend-tools AS frontend
COPY Cargo.toml Cargo.lock Trunk.toml index.html ./
COPY src/ src/
COPY style/ style/
COPY public/ public/
COPY scripts/build.sh scripts/trunk.sh scripts/externalize-bootstrap.py scripts/check-pages.py scripts/
RUN --mount=type=cache,id=brunch-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=brunch-wasm-target,target=/build/target \
    ./scripts/build.sh --public-url / \
    && python3 scripts/check-pages.py --dist dist --public-url / \
    && mkdir -p /out \
    && cp -a dist /out/dist
RUN cd /out && find dist -type f -exec sha256sum {} + | LC_ALL=C sort -k2 > assets.sha256

FROM toolchain AS backend
COPY server/Cargo.toml server/Cargo.lock server/
COPY server/src/ server/src/
COPY server/migrations/ server/migrations/
RUN --mount=type=cache,id=brunch-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=brunch-backend-target,target=/build/server/target \
    cargo build --locked --release --manifest-path server/Cargo.toml \
    && mkdir -p /out \
    && cp server/target/release/brunch-server /out/brunch-server

FROM debian:bookworm-slim AS runtime
ARG SOURCE_REVISION=unknown
LABEL org.opencontainers.image.title="Brunch" \
      org.opencontainers.image.revision="$SOURCE_REVISION"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 brunch \
    && useradd --uid 10001 --gid brunch --no-create-home --shell /usr/sbin/nologin brunch
WORKDIR /app
COPY --from=backend /out/brunch-server /usr/local/bin/brunch-server
COPY --from=frontend /out/dist/ /app/dist/
COPY --from=frontend /out/assets.sha256 /app/assets.sha256
ENV BIND_ADDR=0.0.0.0:3100 DIST_DIR=/app/dist
USER 10001:10001
EXPOSE 3100
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl --fail --silent --max-time 4 http://127.0.0.1:3100/api/health >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/brunch-server"]
