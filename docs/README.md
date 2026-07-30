# VOS-RS 项目文档索引

本目录收录 VOS-RS 平台的全部架构设计、开发规范与集成对接文档。

---

## 📁 目录结构

```
docs/
├── architecture/          # 系统架构设计与技术方案文档
│   ├── ARCHITECTURE.md                        # 整体软件架构与分层设计
│   ├── B2BUA_SESSION_MODEL.md                 # B2BUA session_id 主键模型（A/B-leg 统一会话索引）
│   ├── MULTI_TENANT_DESIGN.md                 # 多租户架构设计（商户关联费率，分机关联商户）
│   ├── RWI_DESIGN.md                          # RWI 实时控制台设计（WebSocket 双工通道 + NATS 事件链路）
│   ├── VOS_RS_ARCHITECTURE_ANALYSIS.md        # 与商业竞品的详细架构分析对比
│   ├── rtp-sip-completeness.md                # RTP/SIP 协议完整性评估与性能基线
│   ├── NATS_VCI_COMMAND_DESIGN.md             # NATS 会话控制协议与 VCI 2.0 命令设计规范
│   ├── TRUNK_CALLER_TERMINATION_DESIGN.md     # 接入认证、主叫号码池与落地决策设计
│   ├── TRUNK_FLOWCHART.md                     # 中继设计与呼叫选路流程图
│   ├── ROUTING_TRUNK_BUSINESS_LOGIC.md        # 中继、路由与号码池关联业务逻辑说明
│   └── VOS_RS_BUSINESS_GAPS_REQUIREMENTS.md   # 业务缺失与后续开发需求 (PRD)
│
├── development/           # 开发与集成接入指南
│   ├── ENV_VARS.md                            # 配置文件架构与环境变量参考
│   ├── AI_PLUGIN_INTEGRATION_GUIDE.md         # AI 语音插件二进制流协议标准（336 字节帧）
│   ├── FRONTEND_OPTIMIZATION.md               # 前端重构与样式优化记录（HeroUI v2 + Tailwind v4）
│   ├── PERFORMANCE_BENCHMARK.md               # SIP 信令与媒体性能基准测试报告
│   ├── SIPP_BUSINESS_SCENARIOS.md             # SIPp 中继与号码业务验证用例
│   └── WEBHOOKS.md                            # Webhook 呼叫事件协议
│
├── deployment/            # 部署与运维
│   ├── DEPLOY.md                              # 生产环境部署指南（Docker Compose）
│   ├── CLUSTER_DEPLOYMENT.md                  # 集群部署（多节点 + 共享状态）
│   └── OS_KERNEL_TUNING.md                    # 操作系统与内核参数调优
│
└── user-guide/            # 用户操作指南
    ├── WEB_GUIDE.md                           # Web 管理界面操作手册
    ├── ROUTING_TRUNK_GUIDE.md                 # 中继与路由管理配置指南
    └── ROUTING_TRUNK_TEST_GUIDE.md            # 中继路由测试指南与 FreeSWITCH 落地对接验证
```

---

## 📖 快速导航

### 架构与设计

| 文档 | 说明 |
|:---|:---|
| [ARCHITECTURE.md](./architecture/ARCHITECTURE.md) | 整体平台分层架构、信令面与媒体面分离设计、关键代码路径 |
| [B2BUA_SESSION_MODEL.md](./architecture/B2BUA_SESSION_MODEL.md) | B2BUA `session_id` 主键模型：A/B-leg Call-ID → session_id → media_session 三级索引 |
| [MULTI_TENANT_DESIGN.md](./architecture/MULTI_TENANT_DESIGN.md) | 多租户架构：域→租户映射、运行时策略快照、计费账户关联、零侵入降级 |
| [RWI_DESIGN.md](./architecture/RWI_DESIGN.md) | RWI 实时控制台：NATS 双主题事件链路、WebSocket 双工通道、媒体控制指令 |
| [VOS_RS_ARCHITECTURE_ANALYSIS.md](./architecture/VOS_RS_ARCHITECTURE_ANALYSIS.md) | 与昆石 VOS 的逐项对比分析，含模块清单与已实现/差距项 |
| [rtp-sip-completeness.md](./architecture/rtp-sip-completeness.md) | RTP/SIP 协议覆盖范围评估、本机性能基线与演进路线图 |
| [NATS_VCI_COMMAND_DESIGN.md](./architecture/NATS_VCI_COMMAND_DESIGN.md) | VCI 2.0 基于 NATS 的同步交互式控制与带外异步指令设计规范 |
| [TRUNK_CALLER_TERMINATION_DESIGN.md](./architecture/TRUNK_CALLER_TERMINATION_DESIGN.md) | 接入认证、主叫号码池、唯一号码归属、分机与落地决策设计 |
| [TRUNK_FLOWCHART.md](./architecture/TRUNK_FLOWCHART.md) | 中继设计与呼叫选路流程图（接入中继 vs 落地中继） |
| [VOS_RS_BUSINESS_GAPS_REQUIREMENTS.md](./architecture/VOS_RS_BUSINESS_GAPS_REQUIREMENTS.md) | 业务缺失与后续开发需求 (PRD)：SipFlow、VCI、WebRTC、Queues |

### 开发与集成

| 文档 | 说明 |
|:---|:---|
| [ENV_VARS.md](./development/ENV_VARS.md) | 统一引导配置文件 `config.yaml` 架构与 `VOS_RS_CONFIG_FILE` 环境变量 |
| [AI_PLUGIN_INTEGRATION_GUIDE.md](./development/AI_PLUGIN_INTEGRATION_GUIDE.md) | AI 语音插件二进制流协议标准（16字节头 + 320字节 PCM16），含 Python/Go 示例 |
| [FRONTEND_OPTIMIZATION.md](./development/FRONTEND_OPTIMIZATION.md) | 前端重构记录：HeroUI v2 + Tailwind v4 升级、按功能域拆分、语义色规范 |
| [PERFORMANCE_BENCHMARK.md](./development/PERFORMANCE_BENCHMARK.md) | SIP 信令与媒体性能基准压测报告（SIPp + RTP relay） |
| [SIPP_BUSINESS_SCENARIOS.md](./development/SIPP_BUSINESS_SCENARIOS.md) | SIPp 中继与号码业务验证用例矩阵（透传/固定/号码池/分机/DID） |
| [WEBHOOKS.md](./development/WEBHOOKS.md) | Webhook 呼叫事件协议（监听型最小闭环 + 事件信封格式） |

### 部署与运维

| 文档 | 说明 |
|:---|:---|
| [DEPLOY.md](./deployment/DEPLOY.md) | Docker Compose 生产环境快速部署流程 |
| [CLUSTER_DEPLOYMENT.md](./deployment/CLUSTER_DEPLOYMENT.md) | 集群部署（多节点 + Redis 共享状态 + NATS 跨节点同步） |
| [OS_KERNEL_TUNING.md](./deployment/OS_KERNEL_TUNING.md) | 操作系统与内核参数调优（UDP buffer、文件句柄、网络栈） |

### 用户操作指南

| 文档 | 说明 |
|:---|:---|
| [WEB_GUIDE.md](./user-guide/WEB_GUIDE.md) | Web 管理界面功能操作手册（14 个页面） |
| [ROUTING_TRUNK_GUIDE.md](./user-guide/ROUTING_TRUNK_GUIDE.md) | 中继与路由管理配置指南（接入中继、落地中继与 LCR 路由决策） |
