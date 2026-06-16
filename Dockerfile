FROM rust:1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml README.md LICENSE ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/media-maintenance /usr/local/bin/media-maintenance
VOLUME ["/data", "/media"]
ENTRYPOINT ["media-maintenance"]
