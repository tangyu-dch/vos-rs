# deploy — 部署配置

> **Docker / systemd / Nginx / Kamailio / 监控 一站式部署配置**

## 这是什么？

`deploy/` 是 vos-rs 项目的 **部署配置** 目录。包含从单机开发到集群生产的全套部署方案。

## 目录结构

```
deploy/
├── docker/              # Docker 部署
│   ├── Dockerfile
│   ├── docker-compose.yml          # 单机部署
│   ├── docker-compose.prod.yml     # 生产部署
│   └── config.compose.yaml         # Compose 配置
├── systemd/             # systemd 服务单元
│   ├── sip-edge.service
│   ├── api-server.service
│   ├── cdr-worker.service
│   ├── media-edge.service
│   └── sip-router.service
├── nginx/               # Nginx 反向代理
│   └── vos-rs.conf
├── kamailio/            # Kamailio SIP 代理 (可选)
│   ├── kamailio.cfg
│   ├── dispatcher.list
│   └── README.md
├── prometheus/          # Prometheus 监控
│   └── prometheus.yml
├── grafana/             # Grafana 仪表盘
│   └── dashboards/
│       └── vos-rs-overview.json
├── helm/                # Helm Chart (Kubernetes)
│   └── vos-rs/
│       ├── Chart.yaml
│       ├── values.yaml
│       └── templates/
├── deploy.sh            # 一键部署脚本
└── docker-compose.env.example  # 环境变量示例
```

## 部署方式

### 1. Docker Compose（推荐开发/小规模生产）

```bash
cd docker
cp docker-compose.env.example .env
# 编辑 .env 配置数据库密码等
docker compose -f docker-compose.yml up -d
```

生产环境：

```bash
cp ../docker-compose.env.example .env
# 创建生产 config.yaml，并使 PostgreSQL、Redis、NATS、RustFS 凭据与 .env 一致。
# Redis/NATS 连接串必须携带认证信息，生产覆盖会拒绝缺失的密码变量。
docker compose -f docker-compose.yml -f docker-compose.prod.yml config --quiet
docker compose -f docker-compose.yml -f docker-compose.prod.yml up -d
```

### 2. systemd（裸机部署）

```bash
# 编译 release
cargo build --workspace --release

# 复制服务单元
sudo cp systemd/*.service /etc/systemd/system/

# 启动
sudo systemctl enable --now sip-edge api-server cdr-worker

# 查看日志
sudo journalctl -u sip-edge -f
```

### 3. Kubernetes（Helm）

```bash
cd helm
helm install vos-rs ./vos-rs -f vos-rs/values.yaml
```

### 4. Kamailio + sip-edge 集群

见 [kamailio/README.md](./kamailio/README.md)。

## 服务端口

| 服务 | 端口 | 协议 | 说明 |
| :--- | :--- | :--- | :--- |
| sip-edge | 5060 | UDP/TCP | SIP 信令 |
| sip-edge | 5062 | TCP | WebSocket |
| sip-edge | 8082 | TCP | 管理 API |
| api-server | 8080 | TCP | REST API |
| sip-router | 5060 | UDP/TCP | 集群 SIP 代理 |
| media-edge | 3030 | TCP | 媒体控制 API |
| media-edge | 40000+ | UDP | RTP 媒体 |
| Prometheus | 9090 | TCP | 监控 |
| Grafana | 3000 | TCP | 仪表盘 |

## Nginx 反向代理

`nginx/vos-rs.conf` 配置：
- 前端静态文件（`web/dist/`）
- API 反向代理到 `api-server:8080`
- WebSocket 升级（SIP over WS）

## 监控

### Prometheus

`prometheus/prometheus.yml` 抓取目标：
- `sip-edge:8082/metrics`
- `api-server:8080/metrics`
- `media-edge:3030/metrics`

### Grafana

`grafana/dashboards/vos-rs-overview.json` 预置仪表盘：
- 并发通话数
- CPS
- 网关健康状态
- RTP 丢包率
- API 延迟

生产覆盖会把 SIP 节点尚未回放的 CDR 挂载到 `vos_rs_cdr_spool` 持久卷；备份时应与 PostgreSQL、NATS 数据卷一并纳入。

## 内核调优

高并发部署需调优内核参数，见 [../docs/deployment/OS_KERNEL_TUNING.md](../docs/deployment/OS_KERNEL_TUNING.md)：

```bash
sysctl -w net.core.rmem_max=16777216
sysctl -w net.core.wmem_max=16777216
sysctl -w net.core.somaxconn=65535
sysctl -w net.ipv4.udp_mem='262144 327680 393216'
```

## 相关文档

- 部署指南：[../docs/deployment/DEPLOY.md](../docs/deployment/DEPLOY.md)
- 集群部署：[../docs/deployment/CLUSTER_DEPLOYMENT.md](../docs/deployment/CLUSTER_DEPLOYMENT.md)
- 内核调优：[../docs/deployment/OS_KERNEL_TUNING.md](../docs/deployment/OS_KERNEL_TUNING.md)
- Kamailio 方案：[./kamailio/README.md](./kamailio/README.md)
