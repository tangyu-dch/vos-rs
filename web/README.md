# VOS-RS Web 管理控制台

基于 Arco Design React 的 VOS-RS VoIP 运营平台管理前端。

## 功能特性

- 📊 **仪表板** - 实时呼叫统计、质量指标和趋势图表
- 📞 **呼叫记录** - 查看和筛选 CDR 记录，包含详细质量信息
- 👤 **SIP 用户管理** - 管理 SIP 用户账户
- 🔌 **网关管理** - 配置 SIP 网关
- 🛣️ **路由管理** - 配置呼叫路由规则
- 📱 **注册信息** - 查看在线设备注册状态

## 技术栈

- React 18
- TypeScript
- Arco Design React
- React Router v6
- ECharts
- Vite
- Axios

## 项目结构

```
web/
├── src/
│   ├── components/         # 公共组件
│   │   ├── Layout.tsx     # 主布局组件
│   │   └── Layout.css
│   ├── pages/             # 页面组件
│   │   ├── Dashboard/     # 仪表板
│   │   ├── Cdr/          # 呼叫记录
│   │   ├── Users/        # SIP 用户
│   │   ├── Gateways/     # 网关管理
│   │   ├── Routes/       # 路由管理
│   │   └── Registrations/ # 注册信息
│   ├── services/          # API 服务
│   │   └── api.ts
│   ├── types/             # TypeScript 类型定义
│   │   └── index.ts
│   ├── App.tsx            # 主应用组件
│   ├── main.tsx           # 入口文件
│   └── index.css
├── index.html
├── package.json
├── tsconfig.json
├── vite.config.ts
└── README.md
```

## 快速开始

### 前置要求

- Node.js 18+
- PostgreSQL 14+ (用于后端存储)
- Rust 1.70+ (用于后端 API 服务)

### 1. 启动后端 API 服务

首先，确保你有一个运行的 PostgreSQL 数据库。

```bash
# 在项目根目录
cd vos-rs

# 设置环境变量
export DATABASE_URL=postgres://user:password@localhost/vos_rs

# 启动 API 服务
cargo run -p api-server
```

API 服务将在 `http://localhost:8080` 启动。

### 2. 启动前端开发服务器

```bash
cd web

# 安装依赖
npm install

# 启动开发服务器
npm run dev
```

访问 `http://localhost:3000` 查看应用。

### 3. 构建生产版本

```bash
cd web
npm run build
```

构建产物将输出到 `web/dist` 目录。

## 后端 API

### 接口列表

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | /health | 健康检查 |
| GET | /api/dashboard/stats | 获取仪表板统计 |
| GET | /api/cdrs | 获取呼叫记录列表 |
| GET | /api/cdrs/:call_id | 获取单个呼叫详情 |
| GET | /api/cdrs/:call_id/dtmf | 获取 DTMF 事件 |
| GET | /api/users | 获取用户列表 |
| POST | /api/users | 创建用户 |
| PUT | /api/users/:username | 更新用户 |
| DELETE | /api/users/:username | 删除用户 |
| GET | /api/gateways | 获取网关列表 |
| POST | /api/gateways | 创建网关 |
| PUT | /api/gateways/:id | 更新网关 |
| DELETE | /api/gateways/:id | 删除网关 |
| GET | /api/routes | 获取路由列表 |
| POST | /api/routes | 创建路由 |
| PUT | /api/routes/:id | 更新路由 |
| DELETE | /api/routes/:id | 删除路由 |
| GET | /api/registrations | 获取注册信息 |

### API 请求/响应示例

#### 获取仪表板统计

```bash
GET /api/dashboard/stats
```

响应：
```json
{
  "active_calls": 5,
  "today_total_calls": 156,
  "today_answered_calls": 132,
  "answer_rate": 84.6,
  "avg_mos": 4.1,
  "avg_loss_rate": 0.2,
  "avg_jitter_ms": 4.5,
  "registered_users": 23,
  "active_gateways": 2
}
```

#### 获取呼叫记录

```bash
GET /api/cdrs?page=1&page_size=20&status=answered
```

响应：
```json
{
  "items": [
    {
      "call_id": "call-1@example.com",
      "caller": "sip:1001@example.com",
      "callee": "13800138000",
      "started_at_ms": 1720000000000,
      "duration_ms": 60000,
      "status": "answered",
      "mos": 4.2
    }
  ],
  "total": 156,
  "page": 1,
  "page_size": 20
}
```

## 环境配置

### 前端配置

在 `web/vite.config.ts` 中可以配置 API 代理：

```typescript
server: {
  port: 3000,
  proxy: {
    '/api': {
      target: 'http://localhost:8080',
      changeOrigin: true
    }
  }
}
```

### 后端配置

使用环境变量配置后端：

| 变量 | 说明 | 默认值 |
|------|------|--------|
| DATABASE_URL | PostgreSQL 连接字符串 | postgres://localhost/vos_rs |

## 数据库表结构

### call_cdrs

| 列 | 类型 | 说明 |
|----|------|------|
| id | BIGSERIAL | 主键 |
| call_id | TEXT | 呼叫 ID |
| caller | TEXT | 主叫 |
| callee | TEXT | 被叫 |
| started_at | TIMESTAMPTZ | 开始时间 |
| answered_at | TIMESTAMPTZ | 应答时间 |
| ended_at | TIMESTAMPTZ | 结束时间 |
| duration_ms | BIGINT | 时长 |
| billable_duration_ms | BIGINT | 计费时长 |
| status | TEXT | 状态 |
| failure_status_code | INTEGER | 失败状态码 |
| failure_reason | TEXT | 失败原因 |
| caller_rtcp_loss_rate | DOUBLE PRECISION | 主叫丢包率 |
| caller_rtcp_jitter_ms | DOUBLE PRECISION | 主叫抖动 |
| caller_rtcp_rtt_ms | INTEGER | 主叫 RTT |
| gateway_rtcp_loss_rate | DOUBLE PRECISION | 网关丢包率 |
| gateway_rtcp_jitter_ms | DOUBLE PRECISION | 网关抖动 |
| gateway_rtcp_rtt_ms | INTEGER | 网关 RTT |
| mos | DOUBLE PRECISION | MOS 值 |
| dtmf_digits | TEXT | DTMF 按键 |
| inserted_at | TIMESTAMPTZ | 插入时间 |

### sip_users

| 列 | 类型 | 说明 |
|----|------|------|
| username | TEXT | 用户名 (主键) |
| password | TEXT | 密码 |
| created_at | TIMESTAMPTZ | 创建时间 |

### sip_gateways

| 列 | 类型 | 说明 |
|----|------|------|
| id | TEXT | 网关 ID (主键) |
| host | TEXT | 主机地址 |
| port | INTEGER | 端口 |
| transport | TEXT | 传输协议 |
| max_capacity | INTEGER | 最大容量 |
| created_at | TIMESTAMPTZ | 创建时间 |

### sip_routes

| 列 | 类型 | 说明 |
|----|------|------|
| id | TEXT | 路由 ID (主键) |
| prefix | TEXT | 前缀 |
| priority | INTEGER | 优先级 |
| gateway_id | TEXT | 网关 ID (外键) |
| cost | DOUBLE PRECISION | 成本 |
| created_at | TIMESTAMPTZ | 创建时间 |

### sip_registrations

| 列 | 类型 | 说明 |
|----|------|------|
| aor | TEXT | AOR |
| contact_uri | TEXT | 联系地址 |
| received_from | TEXT | 来源地址 |
| expires_at | TIMESTAMPTZ | 过期时间 |
| path | TEXT | Path |
| updated_at | TIMESTAMPTZ | 更新时间 |

## 开发说明

### 添加新页面

1. 在 `src/pages/` 下创建新页面组件
2. 在 `src/App.tsx` 中添加路由
3. 在 `src/components/Layout.tsx` 中添加菜单项

### 自定义主题

参考 [Arco Design 主题定制](https://arco.design/react/docs/theme) 文档。

## 许可证

Proprietary
