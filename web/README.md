# VOS-RS Web 管理控制台

## 这是什么？

VOS-RS Web 管理控制台是 vos-rs 电信级 VoIP 软交换平台的统一前端，提供从实时通话监控、号码与中继配置，到 IVR 呼叫中心、计费与路由策略、AI 运维助手、安全防护的端到端运营能力。前端基于 React 18 + TypeScript + Vite 5 构建，UI 采用 HeroUI v2.8 + Tailwind CSS v4，对接后端 `/api/v1/` RESTful 接口（JWT 鉴权）。

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| UI 框架 | React | 18.3 |
| 主语言 | TypeScript | ≥ 5.3 |
| 构建工具 | Vite | 5.3+ |
| UI 组件库 | HeroUI（@heroui/react） | 2.8+ |
| CSS 框架 | Tailwind CSS | 4.3+ |
| 路由 | React Router DOM | 6.25+ |
| HTTP 客户端 | Axios | 1.7+ |
| 图表 | ECharts | 5.5+ |
| 通知 | sonner | 1.7+ |
| 图标 | lucide-react | 1.25+ |
| 动效 | framer-motion | 12+ |
| Markdown 渲染 | react-markdown + remark-gfm | 10+ / 4+ |
| 单元测试 | Vitest + Testing Library | 2.1+ |
| E2E 测试 | Playwright | 1.55+ |

## 功能页面一览

| 分组 | 路由 | 页面 | 说明 |
|------|------|------|------|
| 运营监控 | `/overview` | 仪表盘 | 实时并发/CPS/话费/中继状态 |
| | `/calls/active` | 活跃通话 | 在线通话监控/强制挂断 |
| | `/calls` | 呼叫记录 | CDR 话单查询 |
| | `/calls/:id` | 呼叫详情 | 单次呼叫信令梯形图/媒体质量 |
| | `/copilot` | AI 运维助手 | LLM 自然语言排障/SIP 梯形图 |
| 号码管理 | `/extensions` | 分机管理 | SIP 用户 CRUD |
| | `/extensions/:id` | 分机详情 | 单分机配置/订阅状态 |
| | `/numbers` | 号码管理 | DID 号码 |
| | `/caller-pools` | 主叫号码池 | 号码池分配 |
| | `/caller-pools/:id` | 号码池详情 | 池内号码列表 |
| | `/did-destinations` | DID 目的地 | DID 映射 |
| 中继管理 | `/trunks/access` | 接入中继 | 入局中继 |
| | `/trunks/egress` | 落地中继 | 出局中继 |
| | `/trunks/:id` | 中继详情 | 单中继配置/健康状态 |
| | `/egress-groups` | 落地分组 | 中继分组/负载均衡 |
| | `/egress-groups/:id` | 落地分组详情 | 组内中继编排 |
| 呼叫中心 | `/ivr` | IVR 编辑器 | 可视化拖拽画板 |
| | `/queues` | 队列管理 | ACD 排队策略 |
| | `/agents` | 坐席管理 | 坐席状态/技能组 |
| 路由计费 | `/routing` | 路由配置 | LCR 路由规则 |
| | `/billing/accounts` | 计费账户 | 余额/预扣费 |
| | `/billing/rates` | 费率管理 | 前缀费率 |
| | `/billing/transactions` | 交易记录 | 充值/扣费流水 |
| 系统设置 | `/settings` | 系统设置 | SIP/媒体/录音/TLS 参数 |
| | `/settings/llm` | LLM 配置 | 大模型厂商管理 |
| | `/security` | 安全防护 | SBC/ACL/反欺诈 |
| | `/infrastructure` | 基础设施 | 节点/集群/Redis |

> 未匹配路由会重定向到 `/overview`；权限不足时由 `canAccessPage(session.role, path)` 拦截并跳回 `/overview`。路由定义见 `src/App.tsx`。

## 项目结构

```
web/
├── src/
│   ├── pages/            # 14+ 页面组件
│   │   ├── Login/        # 登录页
│   │   ├── operations/   # 仪表盘/活跃通话/Copilot/呼叫详情
│   │   ├── numbers/      # 分机/号码/号码池/DID 目的地
│   │   ├── trunks/       # 接入/落地中继/落地分组
│   │   ├── call-center/  # IVR/队列/坐席
│   │   ├── billing/      # 账户/费率/交易/呼叫记录
│   │   ├── system/       # 路由/安全/基础设施/设置
│   │   ├── settings/     # LLM 配置
│   │   └── shared/       # 跨页面共享详情/格式化工具
│   ├── components/       # ConsoleShell/detail-shell/ErrorBoundary
│   │   └── ivr/          # IVR 可视化画板节点/表单
│   ├── services/         # API 客户端 + auth/resources 等
│   ├── auth/             # AuthContext（会话/角色）
│   ├── hooks/            # usePageVisibility
│   ├── theme/            # ThemeContext（暗黑/浅色）
│   ├── utils/            # charts/sip/toast
│   ├── types/            # TypeScript 类型定义
│   ├── test/             # 单元测试 + setup
│   ├── App.tsx           # 路由定义 + 权限守卫
│   ├── hero.ts           # HeroUI 主题映射
│   ├── main.tsx          # 入口（ThemeProvider 注入）
│   └── index.css         # Tailwind 入口
├── Dockerfile
├── nginx.conf
├── package.json
├── tsconfig.json
└── vite.config.ts
```

## 快速开始

### 前置要求

- Node.js 18+
- Rust ≥ 1.89（用于后端服务）
- PostgreSQL 14+（主数据 + CDR）

### 1. 启动后端 API

```bash
# 项目根目录
export VOS_RS_DATABASE_URL=postgres://user:password@localhost:5432/vosrs
export VOS_RS_NATS_URL=nats://localhost:4222
cargo run -p api-server
```

API 服务默认监听 `http://localhost:8081`。

### 2. 启动前端开发服务器

```bash
cd web
npm install
npm run dev
```

默认访问 `http://localhost:5173`，Vite 代理将 `/api/v1` 转发到后端。

### 3. 生产构建

```bash
cd web
npm run build        # 产物输出到 web/dist
npm run preview      # 本地预览生产包
```

## 后端 API 对接

### 接口前缀与鉴权

- 所有业务接口前缀：`/api/v1/`
- 鉴权方式：**JWT Bearer Token**（除 `/metrics`、`/api/v1/auth/login` 外均需鉴权）
- 登录获取 Token：

  ```http
  POST /api/v1/auth/login
  Content-Type: application/json

  { "username": "admin", "password": "******" }
  ```

  响应：

  ```json
  { "code": 0, "message": "success", "data": { "access_token": "eyJhbGciOi..." } }
  ```

- 请求头需携带 `Authorization: Bearer <access_token>`，由 `services/client.ts` 拦截器自动注入。
- 401 响应会触发会话清理并跳转 `/login`。

### 统一响应格式

```json
{
  "code": 0,
  "message": "success",
  "data": { },
  "timestamp": 1720000000,
  "request_id": "req_xxxxxxxxxxxx"
}
```

错误响应额外附带 `details` 字段说明失败原因。

### 主要端点（按页面分组）

| 路径 | 方法 | 说明 |
|------|------|------|
| `/api/v1/auth/login` | POST | 登录获取 JWT |
| `/api/v1/dashboard/stats` | GET | 仪表盘统计 |
| `/api/v1/active-calls` | GET | 活跃通话列表 |
| `/api/v1/cdr` | GET | CDR 话单查询（注意：单数为 `cdr`） |
| `/api/v1/copilot/sessions/:id/chat/stream` | GET(SSE) | Copilot 流式对话 |
| `/api/v1/users` | CRUD | SIP 用户管理 |
| `/api/v1/gateways` | CRUD | 网关管理 |
| `/api/v1/routes` | CRUD | 路由管理 |
| `/api/v1/numbers` | CRUD | 号码管理 |
| `/api/v1/rates` | CRUD | 费率管理 |
| `/api/v1/billing/accounts` | CRUD | 计费账户 |
| `/api/v1/billing/transactions` | GET | 交易流水 |
| `/api/v1/recordings` | GET | 录音查询 |
| `/api/v1/registrations` | GET | 注册状态 |
| `/api/v1/anti-fraud/rules` | CRUD | 反欺诈规则 |
| `/metrics` | GET | Prometheus 指标（无需鉴权） |

### SSE 流式接口

Copilot 对话通过 `EventSource` 消费 `/api/v1/copilot/sessions/:id/chat/stream`，逐 token 推送 `text/event-stream` 事件，前端在 `pages/operations/copilot*` 系列组件中聚合渲染。

## 环境配置

### 前端

```bash
# web/.env.local
VITE_API_BASE_URL=http://localhost:8081   # 后端 API 地址
VITE_WS_BASE_URL=ws://localhost:8081      # WebSocket 地址（可选）
```

Vite 开发服务器默认通过 `vite.config.ts` 中的 `proxy` 把 `/api/v1` 转发到 `VITE_API_BASE_URL`，前端代码无需感知绝对域名。

### 后端（节选，完整列表见 `docs/development/ENV_VARS.md`）

| 变量 | 说明 |
|------|------|
| `VOS_RS_DATABASE_URL` | PostgreSQL 连接串 |
| `VOS_RS_NATS_URL` | NATS JetStream 地址 |
| `VOS_RS_SIP_BIND` | SIP 监听地址 |
| `VOS_RS_AUTH_ENABLED` | 是否启用 SIP Digest Auth |
| `VOS_RS_RTP_PORT_MIN` / `VOS_RS_RTP_PORT_MAX` | RTP 端口范围 |
| `VOS_RS_RECORDING_ENABLED` | 录音开关 |
| `VOS_RS_SBC_ALLOW` / `VOS_RS_SBC_BLOCK` | SBC IP 白/黑名单（CIDR） |

> 注意：所有后端变量统一以 `VOS_RS_` 为前缀，与历史文档中的 `DATABASE_URL` 不再兼容。

## 主题系统

- `ThemeProvider` 通过 `localStorage` 持久化用户偏好，键名为 `vos-theme`，值为 `dark` 或 `light`。
- HeroUI 通过 `<html class="dark">` 切换暗色主题；同时设置 `root.style.colorScheme` 以兼容原生控件。
- 默认主题：**light**（在 `src/theme/ThemeContext.tsx` 的 `readStoredTheme` 中可修改回退默认值）。
- 切换 API：`const { theme, toggleTheme, setTheme } = useTheme();`。

## 测试

### 单元测试（Vitest）

```bash
cd web
npm test             # 运行所有 *.test.ts(x)
npm run test -- --watch
```

测试覆盖 `services/`（API 客户端、auth）、`utils/`（charts、sip、toast）、`pages/`（call-detail、trunks、resource-form、termination-config）等模块，运行环境为 jsdom，setup 文件位于 `src/test/setup.ts`。

### E2E 测试（Playwright）

```bash
cd web
npm run test:e2e              # 运行端到端用例
npm run test:e2e:report       # 打开上次报告
```

## 架构概览

### 前端导航结构

```mermaid
flowchart LR
  Login --> ConsoleShell
  ConsoleShell --> Overview[运营监控]
  ConsoleShell --> Numbers[号码管理]
  ConsoleShell --> Trunks[中继管理]
  ConsoleShell --> CC[呼叫中心]
  ConsoleShell --> Routing[路由计费]
  ConsoleShell --> System[系统设置]

  Overview --> Dashboard[仪表盘]
  Overview --> Active[活跃通话]
  Overview --> Calls[呼叫记录]
  Overview --> Copilot[AI 运维助手]

  Numbers --> Extensions[分机]
  Numbers --> DidNum[号码/DID]
  Numbers --> Pools[主叫号码池]

  Trunks --> Access[接入中继]
  Trunks --> Egress[落地中继]
  Trunks --> Groups[落地分组]

  CC --> IVR[IVR 编辑器]
  CC --> Queues[队列]
  CC --> Agents[坐席]

  Routing --> Routes[路由配置]
  Routing --> Accounts[计费账户]
  Routing --> Rates[费率]
  Routing --> Txn[交易记录]

  System --> Settings[系统设置]
  System --> LLM[LLM 配置]
  System --> Sec[安全防护]
  System --> Infra[基础设施]
```

### 组件架构

```mermaid
flowchart TD
  ThemeProvider --> ErrorBoundary
  AuthProvider --> ErrorBoundary
  ErrorBoundary --> App[App.tsx 路由]
  App --> ConsoleShell
  ConsoleShell --> Pages[pages/* 页面组件]
  Pages --> Shared[shared/ 详情与表单]
  Pages --> Services[services/ API 客户端]
  Services --> Client[client.ts axios + JWT 拦截器]
  Client --> Backend[/api/v1/ 后端]
  Pages --> Theme[useTheme]
  Pages --> Hooks[hooks/ usePageVisibility]
  Pages --> Components[components/ ConsoleShell/IVR/SipTrace]
```

## 相关文档

| 文档 | 位置 |
|------|------|
| 项目 AI 指南 | `AGENTS.md` |
| 架构分析 | `docs/architecture/VOS_RS_ARCHITECTURE_ANALYSIS.md` |
| 系统架构 | `docs/architecture/ARCHITECTURE.md` |
| 环境变量参考 | `docs/development/ENV_VARS.md` |
| 部署指南 | `docs/deployment/DEPLOY.md` |
| Web 界面使用指南 | `docs/user-guide/WEB_GUIDE.md` |
| SIP/RTP 完整性 | `docs/architecture/rtp-sip-completeness.md` |

## 许可证

Proprietary
