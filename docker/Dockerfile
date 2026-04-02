# stage 1: builder
FROM rust:1.77-slim AS builder

WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p frostmirror && \
    cp target/release/frostmirror /frostmirror-bin

# stage 2: runtime (~15 MB final image)
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /frostmirror-bin /usr/local/bin/frostmirror
COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

VOLUME ["/pkgs", "/mirror", "/config"]
EXPOSE 8080

ENTRYPOINT ["/entrypoint.sh"]
CMD ["serve"]
