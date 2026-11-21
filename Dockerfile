FROM node:22-bookworm-slim AS dashboard
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/index.html web/tsconfig.json web/vite.config.ts ./
COPY web/src/ ./src/
RUN npm run build

FROM rust:1.99.1-slim-bookworm AS cli
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml build.rs ./
COPY proto/flock.proto ./proto/
COPY src/ ./src/
COPY examples/local.yaml examples/local.toml ./examples/
COPY --from=dashboard /web/dist/index.html ./web/dist/index.html
ARG STARLING_FEATURES=""
RUN cargo build --locked --release --features "$STARLING_FEATURES"

FROM debian:bookworm-slim
COPY --from=cli /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=cli /src/target/release/starling /usr/local/bin/starling
COPY LICENSE /usr/share/doc/starling/LICENSE
WORKDIR /work
RUN chown 65532:65532 /work
USER 65532:65532
ENTRYPOINT ["starling"]
CMD ["--help"]
