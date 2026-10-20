# Keepword node container image. Each release publishes it for x86_64 and
# arm64 as git.keepword.net/keepword/keepword; to build this checkout:
#
#   docker build -t keepword .
#   docker run -d --name keepword -v keepword-data:/data \
#       -p 127.0.0.1:8480:8480 -p 8481:8481 \
#       -e KEEPWORD_ENDPOINT=https://witness.example.org keepword
#
# See packaging/docker/compose.yaml and docs/INSTALL.md for the settings.

# "release" takes the binary from dist/<arch>/keepword, a release's static
# build, instead of compiling it (.forgejo/workflows/release.yml).
ARG BINARY=build

FROM rust:1-trixie AS build
WORKDIR /src
COPY . .
# Set to "render" for headless-browser capture (also install Chromium below).
ARG FEATURES=""
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p keepword ${FEATURES:+--features $FEATURES} \
    && install -m 0755 target/release/keepword /usr/local/bin/keepword

FROM scratch AS release
ARG TARGETARCH
COPY dist/${TARGETARCH}/keepword /usr/local/bin/keepword

FROM ${BINARY} AS binary

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --user-group --home-dir /data --shell /usr/sbin/nologin keepword \
    && install -d -m 0750 -o keepword -g keepword /data
COPY --from=binary /usr/local/bin/keepword /usr/local/bin/keepword
COPY packaging/docker/entrypoint.sh /usr/local/bin/keepword-entrypoint
USER keepword
ENV KEEPWORD_DIR=/data HOME=/data
VOLUME /data
# 8480: web UI (keep private), 8481: peer API (publish behind TLS)
EXPOSE 8480 8481
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s \
    CMD curl -fsS http://127.0.0.1:8481/v1/descriptor >/dev/null || exit 1
ENTRYPOINT ["keepword-entrypoint"]
CMD ["serve", "--addr", "0.0.0.0:8480", "--api-addr", "0.0.0.0:8481", "--watch", "--anchor"]
