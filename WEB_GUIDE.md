# VOS-RS Web 管理界面使用指南

## 项目概述

VOS-RS 现在包含一个完整的 Web 管理界面，使用 React + Arco Design 构建，后端使用 Rust Axum 提供 REST API。

## 项目结构

```
vos-rs/
├── crates/              # 核心库
│   ├── cdr-core/       # CDR 存储和 API 数据模型（已增强）
│   └── ...
├── services/           # 服务
│   ├── sip-edge/       # SIP 边缘服务
│   ├── cdr-worker/     # CDR 工作服务
│   └── api-server/     # 新增：REST API 服务
├── web/                # 新增：Web 前端
│   ├── src/
│   │   ├── components/
│   │   ├── pages/
│   │   ├── services/
│   │   └── ...
│   ├── package.json
│   └── ...
└── ...
```

## 快速开始

### 前置要求

- Rust 1.89+
- PostgreSQL 14+
- Node.js 18+
- npm 或 yarn

### 1. 设置数据库

```bash
# 创建数据库
createdb vos_rs

# 或者使用 Docker
docker run -d --name vos-rs-postgres \
  -e POSTGRES_USER=vos_rs \
  -e POSTGRES_PASSWORD=vos_rs \
  -e POSTGRES_DB=vos_rs \
  -p 5432:5432 \
  postgres:14
```

### 2. 启动后端 API 服务

```bash
# 在项目根目录
export DATABASE_URL=postgres://user:password@localhost/vos_rs

# 启动 API 服务器
cargo run -p api-server
```

API 服务将在 `http://localhost:8080` 启动。

### 3. 启动 Web 前端

```bash
cd web

# 安装依赖
npm install

# 启动开发服务器
npm run dev
```

访问 `http://localhost:3000` 查看管理界面。

## 功能模块

### 1. 仪表板 (Dashboard)

- **实时统计**：显示活跃呼叫、今日总呼叫、接通率
- **质量指标**：平均 MOS、丢包率、抖动
- **趋势图表**：使用 ECharts 展示呼叫趋势
- **最近记录**：显示最近的 CDR 记录

### 2. 呼叫记录 (CDR)

- **列表展示**：分页显示所有呼叫记录
- **筛选功能**：按状态、主叫、被叫、时间筛选
- **详情查看**：查看单个呼叫的完整信息和质量指标
- **DTMF 记录**：显示呼叫过程中的 DTMF 按键

### 3. SIP 用户管理

- **用户列表**：显示所有 SIP 用户
- **创建用户**：添加新的 SIP 用户
- **编辑用户**：更新用户密码
- **删除用户**：删除不再需要的用户

### 4. 网关管理

- **网关列表**：显示所有配置的 SIP 网关
- **创建网关**：添加新的 SIP 网关
- **编辑网关**：更新网关配置
- **删除网关**：删除不再需要的网关

### 5. 路由管理

- **路由列表**：显示所有路由规则
- **创建路由**：添加新的路由规则
- **编辑路由**：更新路由配置
- **删除路由**：删除不再需要的路由

### 6. 注册信息

- **注册列表**：显示当前在线的设备注册
- **状态显示**：显示注册状态（有效/即将过期/已过期）
- **搜索功能**：快速查找特定的注册

## API 接口文档

### 健康检查

```http
GET /health
```

### 仪表板

```http
GET /api/dashboard/stats
```

响应示例：
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

### CDR 记录

```http
# 获取 CDR 列表
GET /api/cdrs?page=1&page_size=20

# 获取单个 CDR
GET /api/cdrs/:call_id

# 获取 CDR 的 DTMF 事件
GET /api/cdrs/:call_id/dtmf
```

### SIP 用户

```http
# 获取用户列表
GET /api/users

# 创建用户
POST /api/users
{
  "username": "1001",
  "password": "secret"
}

# 更新用户
PUT /api/users/:username
{
  "password": "newsecret"
}

# 删除用户
DELETE /api/users/:username
```

### SIP 网关

```http
# 获取网关列表
GET /api/gateways

# 创建网关
POST /api/gateways
{
  "id": "gw1",
  "host": "sip.example.com",
  "port": 5060,
  "transport": "udp",
  "max_capacity": 100
}

# 更新网关
PUT /api/gateways/:id

# 删除网关
DELETE /api/gateways/:id
```

### 路由管理

```http
# 获取路由列表
GET /api/routes

# 创建路由
POST /api/routes
{
  "id": "route1",
  "prefix": "138",
  "priority": 10,
  "gateway_id": "gw1",
  "cost": 0.01
}

# 更新路由
PUT /api/routes/:id

# 删除路由
DELETE /api/routes/:id
```

### 注册信息

```http
# 获取注册列表
GET /api/registrations
```

## 开发指南

### 前端开发

```bash
cd web

# 开发模式
npm run dev

# 类型检查
npm run type-check

# 构建生产版本
npm run build

# 预览生产构建
npm run preview
```

### 后端开发

```bash
# 运行 API 服务器
cargo run -p api-server

# 检查代码格式
cargo fmt --check

# 运行测试
cargo test
```

## 配置说明

### 前端配置

编辑 `web/vite.config.ts` 修改 API 代理：

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

使用环境变量：

```bash
# 数据库连接
export DATABASE_URL=postgres://user:password@localhost/vos_rs
```

## 数据库表结构

系统会自动创建以下表：

- `call_cdrs` - 呼叫详细记录
- `sip_users` - SIP 用户
- `sip_gateways` - SIP 网关
- `sip_routes` - SIP 路由
- `sip_registrations` - SIP 注册信息
- `dtmf_events` - DTMF 事件

## 故障排除

### 数据库连接失败

确保 PostgreSQL 正在运行，并且连接字符串正确：

```bash
export DATABASE_URL=postgres://vos_rs:vos_rs@localhost/vos_rs
```

### 前端无法连接 API

确保 API 服务器正在运行，并且 CORS 配置正确（默认允许所有来源）。

### 端口被占用

修改 API 服务器端口：编辑 `services/api-server/src/main.rs` 中的地址配置。

## 下一步

- 阅读 [web/README.md](web/README.md) 了解更多前端开发细节
- 查看主 [README.md](README.md) 了解 VOS-RS 的完整功能
- 启动 sip-edge 服务进行完整的 SIP 呼叫测试
