# 后端构建：sip-edge + api-server（同一镜像，docker-compose 用 command 区分）
FROM rust:bookworm-slim AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

# 先拷依赖清单利用 Docker 层缓存
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services

RUN cargo build --release -p sip-edge -p api-server

# 运行时镜像（精简）
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/sip-edge /app/sip-edge
COPY --from=builder /app/target/release/api-server /app/api-server

# SIP UDP/TCP、管理 API、API 服务
EXPOSE 5060/udp 5060 8080 8082

# 默认启动 api-server；docker-compose 用 command 覆盖以启动 sip-edge
CMD ["./api-server"]
