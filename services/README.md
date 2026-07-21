# services — vos-rs 可执行服务集合

> **vos-rs 的二进制服务集合** — 把 `crates/` 里的能力组装成可运行的服务进程

## 这是什么？

`services/` 是 vos-rs 项目的 **可执行二进制服务集合**。每个子目录是一个独立的 `cargo` 二进制 crate，编译后生成可执行文件，部署为独立的系统服务（systemd / Docker）。

打个比方：如果 `crates/` 是「零件」，那 `services/` 是「装好的整车」——加好油就能跑。

## 服务清单

| 服务 | 类型 | 一句话定位 | 默认端口 |
| :--- | :--- | :--- | :--- |
| [`sip-edge`](./sip-edge/) | 信令+媒体 | SIP B2BUA 核心，处理呼叫信令 + RTP 媒体中继 | 5060 (SIP) / 8080 (管理) |
| [`api-server`](./api-server/) | REST API | Web 控制台后端 + 30+ 业务端点 | 8081 (HTTP) |
| [`cdr-worker`](./cdr-worker/) | 异步任务 | 从 NATS 消费 CDR 事件批量写入 PostgreSQL | 无（消费者） |
| [`media-edge`](./media-edge/) | 媒体边缘 | 下一代媒体中继，eBPF/XDP 内核旁路加速 | 10000+ (RTP) |
| [`sip-router`](./sip-router/) | SIP 代理 | 集群 SIP 路由器，TCP/UDP 代理 + 对话路由 | 5060 (UDP/TCP) |

## 架构关系

```text
                    ┌─────────────────┐
                    │   web/ (前端)    │
                    └────────┬────────┘
                             │ HTTP API
                             ▼
┌──────────────┐    ┌─────────────────┐    ┌──────────────┐
│  sip-router  │◄──►│   api-server    │◄──►│  cdr-worker  │
│  (集群代理)   │    │   (REST API)    │    │  (CDR 落库)  │
└──────┬───────┘    └────────┬────────┘    └──────┬───────┘
       │                     │                    │
       │ SIP                 │ 配置/CDR           │ NATS 消费
       ▼                     ▼                    │
┌──────────────┐    ┌─────────────────┐           │
│   sip-edge   │───►│   PostgreSQL    │◄──────────┘
│  (B2BUA 核心) │    │   (主数据库)     │
└──────┬───────┘    └─────────────────┘
       │ RTP
       ▼
┌──────────────┐
│  media-edge  │
│ (eBPF 媒体)   │
└──────────────┘
```

## 部署形态

### 1. 单机部署（开发 / 小规模）

所有 5 个服务跑在一台机器上：

```bash
docker compose -f deploy/docker/docker-compose.yml up -d
```

### 2. 集群部署（生产）

- **sip-router** × N 台：前置负载均衡，无状态
- **sip-edge** × N 台：B2BUA，按 CallId 哈希分发
- **media-edge** × N 台：媒体中继，信令媒体分离
- **api-server** × 2 台：Web 控制台，前置 nginx
- **cdr-worker** × 1-2 台：CDR 落库，NATS 消费者组

详见 [../docs/deployment/CLUSTER_DEPLOYMENT.md](../docs/deployment/CLUSTER_DEPLOYMENT.md)。

## 共同约定

### 配置文件

所有服务通过 `config.yaml` 统一配置，环境变量 `VOS_RS_CONFIG_FILE` 指定路径。配置项见 [../docs/development/ENV_VARS.md](../docs/development/ENV_VARS.md)。

### 日志

使用 `tracing` + `tracing-subscriber`，结构化日志，支持 `RUST_LOG` 环境变量分级：

```bash
RUST_LOG=sip_edge=debug,media=trace,info
```

### 错误处理

- 库层（crates）：`thiserror` 定义错误枚举
- 服务层（services）：`anyhow` 聚合上下文 + `Result` 传播
- 禁止 `unwrap()` / `expect()` 在生产代码

### 优雅停机

所有服务监听 `SIGTERM` / `SIGINT`，收到信号后：
1. 停止接受新请求
2. 等待在途通话结束（最长 60 秒）
3. 刷新 CDR 批次
4. 关闭数据库连接
5. 退出进程

## 编译与运行

### 全量编译

```bash
cargo build --workspace              # 开发构建
cargo build --workspace --release    # 生产构建
```

### 单服务编译运行

```bash
cargo run -p sip-edge --release      # 运行 sip-edge
cargo run -p api-server --release    # 运行 api-server
```

### systemd 部署

服务单元文件在 [../deploy/systemd/](../deploy/systemd/)：

```bash
sudo cp deploy/systemd/sip-edge.service /etc/systemd/system/
sudo systemctl enable --now sip-edge
```

### Docker 部署

```bash
docker build -t vos-rs:sip-edge -f deploy/docker/Dockerfile .
docker run -d --name sip-edge -p 5060:5060/udp vos-rs:sip-edge
```

## 性能指标

| 服务 | 单机目标 | 说明 |
| :--- | :--- | :--- |
| `sip-edge` | 5000+ 并发通话 / 1000+ CPS | B2BUA 核心 |
| `media-edge` | 10万路 RTP 转发 | eBPF/XDP 内核旁路 |
| `api-server` | P99 < 100ms | REST API |
| `cdr-worker` | 5000 CDR/s | 批量写入 |
| `sip-router` | 10000+ CPS | 无状态代理 |

## 相关文档

- 项目根 README：[../README.md](../README.md)
- 共享库：[../crates/README.md](../crates/README.md)
- 部署指南：[../docs/deployment/DEPLOY.md](../docs/deployment/DEPLOY.md)
- 集群部署：[../docs/deployment/CLUSTER_DEPLOYMENT.md](../docs/deployment/CLUSTER_DEPLOYMENT.md)
- 环境变量：[../docs/development/ENV_VARS.md](../docs/development/ENV_VARS.md)
- 各服务详细文档：见各服务目录下的 `README.md`
