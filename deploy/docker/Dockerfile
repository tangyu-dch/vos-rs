# 后端构建：所有 Rust 服务共用同一镜像，由 docker-compose 的 command 区分。
# 固定 Rust 主版本，避免使用不存在或随时间漂移的 latest 标签。
FROM rust:1.89-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# 先拷依赖清单利用 Docker 层缓存
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services

# target 是 BuildKit 缓存挂载，必须在同一构建层导出产物。
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release \
    -p sip-router \
    -p sip-edge \
    -p media-edge \
    -p api-server \
    -p cdr-worker \
    && mkdir -p /app/dist \
    && cp /app/target/release/sip-router /app/dist/sip-router \
    && cp /app/target/release/sip-edge /app/dist/sip-edge \
    && cp /app/target/release/media-edge /app/dist/media-edge \
    && cp /app/target/release/api-server /app/dist/api-server \
    && cp /app/target/release/cdr-worker /app/dist/cdr-worker

# 运行时镜像（精简）
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/dist/sip-router /app/sip-router
COPY --from=builder /app/dist/sip-edge /app/sip-edge
COPY --from=builder /app/dist/media-edge /app/media-edge
COPY --from=builder /app/dist/api-server /app/api-server
COPY --from=builder /app/dist/cdr-worker /app/cdr-worker
COPY deploy/docker/config.compose.yaml /etc/vos-rs/config.yaml

# SIP UDP/TCP、WebSocket、媒体控制、管理 API、API 服务
EXPOSE 3030 5060/udp 5060 5062 8080 8082 8083

# 默认启动 api-server；docker-compose 用 command 覆盖以启动 sip-edge
CMD ["./api-server"]
