# syntax=docker/dockerfile:1
# Builds the egui web client with trunk, then the server that embeds it.
# The ArrangeUs format key is not part of the image: give it at runtime (ARUS_FORMAT_KEY).

FROM docker.io/library/rust:1-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
ARG TRUNK_VERSION=0.21.14
RUN curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-$(uname -m)-unknown-linux-gnu.tar.gz" \
    | tar -xz -C /usr/local/bin trunk
RUN rustup target add wasm32-unknown-unknown
ENV CARGO_TARGET_DIR=/target
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=arus-cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=arus-cargo-git \
    --mount=type=cache,target=/target,id=arus-target \
    --mount=type=cache,target=/root/.cache/trunk,id=arus-trunk \
    cd crates/arrus-web && trunk build --release \
    && cd /src && cargo build --release --locked -p arrus-server \
    && cp /target/release/arrus-server /usr/local/bin/arrus-server

FROM docker.io/library/debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/arrus-server /usr/local/bin/arrus-server
ENV ARUS_DATA_DIR=/data ARUS_PORT=8080
EXPOSE 8080
CMD ["arrus-server"]
