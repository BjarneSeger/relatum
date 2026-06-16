# Builds either workspace binary. Select with `--build-arg BIN=relatum-server`
# (default) or `--build-arg BIN=relatum-web`. Both binaries are pure-Rust (rustls,
# no OpenSSL) and bake their assets/templates in, so the runtime stage is identical
# — only the binary copied in differs.
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /relatum-server

FROM chef AS planner
ARG BIN=relatum-server
COPY . .
RUN cargo chef prepare --recipe-path recipe.json --bin ${BIN}

FROM chef AS builder
ARG BIN=relatum-server
COPY --from=planner /relatum-server/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json --bin ${BIN}
# Build application
COPY . .
RUN cargo build --release --package ${BIN}

FROM debian:trixie-slim AS runtime
ARG BIN=relatum-server
WORKDIR /relatum-server
# Copy to a fixed path so the exec-form ENTRYPOINT does not need to expand ${BIN}
# (ARGs are not interpolated in exec-form ENTRYPOINT).
COPY --from=builder /relatum-server/target/release/${BIN} /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]
