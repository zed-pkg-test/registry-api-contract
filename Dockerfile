# Build context must be the PARENT directory (side-by-side checkout) because
# of the ../zed-interfaces path dependency:
#
#   docker build -f zed-api-server.rs/Dockerfile -t ghcr.io/zed-pkg/zed-api-server:dev .
FROM rust:1-slim AS build
WORKDIR /work
COPY zed-interfaces ./zed-interfaces
COPY zed-api-server.rs ./zed-api-server.rs
WORKDIR /work/zed-api-server.rs
RUN cargo build --release --locked 2>/dev/null || cargo build --release

FROM debian:stable-slim
RUN useradd --system --uid 10001 zed
COPY --from=build /work/zed-api-server.rs/target/release/zed-api-server /usr/local/bin/zed-api-server
USER zed
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/zed-api-server"]
