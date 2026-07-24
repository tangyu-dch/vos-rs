# Multi-stage Dockerfile for vos-rs Telecom Softswitch Engine
# Stage 1: Build binaries
FROM rust:1.85-slim-bookworm AS builder

WORKDIR /usr/src/vos-rs
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services

RUN cargo build --release --workspace

# Stage 2: Runtime environment
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates libssl3 netcat-openbsd curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy built binaries
COPY --from=builder /usr/src/vos-rs/target/release/sip-edge /app/sip-edge
COPY --from=builder /usr/src/vos-rs/target/release/api-server /app/api-server
COPY --from=builder /usr/src/vos-rs/target/release/cdr-worker /app/cdr-worker
COPY --from=builder /usr/src/vos-rs/target/release/sip-router /app/sip-router

# Copy static configurations
COPY config.yaml /app/config.yaml

EXPOSE 5060/udp 5060/tcp 8080/tcp 8082/tcp 9090/tcp

CMD ["/app/sip-edge"]
