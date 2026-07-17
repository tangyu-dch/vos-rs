# VOS-RS 项目文档索引

本目录收录 VOS-RS 平台的全部架构设计、开发规范与集成对接文档。

---

## 📁 目录结构

```
docs/
├── architecture/          # 系统架构设计与技术方案文档
│   ├── ARCHITECTURE.md                        # 整体软件架构与分层设计
│   ├── VOS_RS_ARCHITECTURE_ANALYSIS.md        # 与商业竞品的详细架构分析对比
│   ├── rtp-sip-completeness.md                # RTP/SIP 协议完整性评估
│   ├── WEBHOOKS_DESIGN_COMPARISON.md          # Webhooks 全流程控制与监听设计（含指令集对比）
│   ├── WEBHOOKS_EXTENSIBILITY_ARCHITECTURE.md # Webhooks 插拔式可扩展通道与执行器架构
│   ├── NATS_VCI_COMMAND_DESIGN.md             # NATS 会话控制协议与命令设计规范
│   └── TRUNK_CALLER_TERMINATION_DESIGN.md     # 接入认证与主被叫决策设计
│
├── development/           # 开发与集成接入指南
│   ├── ENV_VARS.md                            # 环境变量配置参考
│   └── AI_PLUGIN_INTEGRATION_GUIDE.md         # AI 语音插件标准接入协议对接指南
│
├── deployment/            # 部署与运维
│   └── DEPLOY.md                              # 生产环境部署指南
│
└── user-guide/            # 用户操作指南
    ├── WEB_GUIDE.md                           # Web 管理界面操作手册
    └── ROUTING_TRUNK_GUIDE.md                 # 中继与路由管理配置指南
```

---

## 📖 快速导航

### 架构与设计
 
 | 文档 | 说明 |
 |:---|:---|
 | [WEBHOOKS_DESIGN_COMPARISON.md](./architecture/WEBHOOKS_DESIGN_COMPARISON.md) | VCI 2.0 全栈 12 大指令集设计、事件监听与 RustPBX/Twilio 对比分析 |
 | [WEBHOOKS_EXTENSIBILITY_ARCHITECTURE.md](./architecture/WEBHOOKS_EXTENSIBILITY_ARCHITECTURE.md) | `WebhookTransport` / `VciExecutor` 插拔式多通道可扩展架构设计 |
 | [NATS_VCI_COMMAND_DESIGN.md](./architecture/NATS_VCI_COMMAND_DESIGN.md) | VCI 2.0 基于 NATS 消息队列的同步交互式控制与带外异步指令设计规范 |
 | [TRUNK_CALLER_TERMINATION_DESIGN.md](./architecture/TRUNK_CALLER_TERMINATION_DESIGN.md) | 接入认证、主叫号码池、唯一号码归属、分机与落地决策设计 |
 | [ARCHITECTURE.md](./architecture/ARCHITECTURE.md) | 整体平台分层架构、信令面与媒体面分离设计 |
 
 ### 开发与集成
 
 | 文档 | 说明 |
 |:---|:---|
 | [AI_PLUGIN_INTEGRATION_GUIDE.md](./development/AI_PLUGIN_INTEGRATION_GUIDE.md) | AI 语音插件二进制流协议标准（16字节头 + 320字节PCM16），含 Python/Go 完整示例 |
 | [ENV_VARS.md](./development/ENV_VARS.md) | 所有 `VOS_RS_*` 环境变量的说明与默认值 |
 
 ### 部署与操作
 
 | 文档 | 说明 |
 |:---|:---|
 | [DEPLOY.md](./deployment/DEPLOY.md) | Docker Compose 生产环境快速部署流程 |
 | [ROUTING_TRUNK_GUIDE.md](./user-guide/ROUTING_TRUNK_GUIDE.md) | 中继与路由管理配置指南 (接入中继、落地中继与 LCR 路由决策) |
 | [WEB_GUIDE.md](./user-guide/WEB_GUIDE.md) | Web 管理界面功能操作手册 |
