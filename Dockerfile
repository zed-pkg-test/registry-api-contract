# Build context must be the PARENT directory (side-by-side checkout) because
# of the ../zed-interfaces path dependency:
#
#   docker build -f zed-api-server.rs/Dockerfile -t ghcr.io/zed-pkg/zed-api-server:dev .
#
# Base images are pinned to explicit minor tags (not floating rust:1-slim /
# debian:stable-slim) so rebuilds are reproducible.
FROM rust:1.83-slim AS build
WORKDIR /work
COPY zed-interfaces ./zed-interfaces
COPY zed-api-server.rs ./zed-api-server.rs
WORKDIR /work/zed-api-server.rs
# --locked must fail the build if Cargo.lock is stale; never fall back to an
# unlocked build that could silently pull different dependency versions.
RUN cargo build --release --locked

FROM debian:12-slim
RUN useradd --system --uid 10001 zed
COPY --from=build /work/zed-api-server.rs/target/release/zed-api-server /usr/local/bin/zed-api-server
USER zed
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/zed-api-server", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/zed-api-server"]
