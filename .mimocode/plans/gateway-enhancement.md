# 网关路由增强计划

## 问题分析
1. 13900000000 走 default 路由 — 需要检查路由表配置
2. 网关需要前缀处理能力（添加/剥离/替换）
3. 需要区分对接网关、落地网关、分机

## 实现方案

### 1. 数据库层 (`sip_gateways` 表)

新增字段：
```sql
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS gateway_type VARCHAR(20) DEFAULT 'peer';
-- peer = 对接网关（我们对接其他线路）
-- gateway = 落地网关（别人对接我们）
-- extension = 分机（落地网关 + 注册认证）

ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_add VARCHAR(50) DEFAULT '';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_strip INTEGER DEFAULT 0;
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_replace_from VARCHAR(50) DEFAULT '';
ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_replace_to VARCHAR(50) DEFAULT '';
```

### 2. 路由逻辑层 (`call-core/src/routing.rs`)

**RouteTarget** 新增字段：
- `prefix_add: String` — 添加前缀
- `prefix_strip: usize` — 剥离前N位
- `prefix_replace: (String, String)` — 前缀替换

**outbound_uri_for** 方法增强：
- 在构建 outbound URI 时应用前缀处理
- 处理顺序：strip → replace → add

### 3. 路由试算逻辑

当前问题：13900000000 走 default 是因为没有匹配 "139" 前缀的路由。

解决方案：
- 在路由试算 API 中显示前缀处理后的实际被叫号码
- 在前端显示完整的选路过程

### 4. 前端网关管理

新增字段展示：
- 网关类型标签（对接/落地/分机）
- 前缀处理配置

### 5. 文件变更清单

| 文件 | 变更 |
|------|------|
| `scripts/gateway_enhancement_migration.sql` | 新增 |
| `crates/call-core/src/routing.rs` | RouteTarget 新增前缀字段，outbound_uri_for 增强 |
| `crates/cdr-core/src/lib.rs` | sip_gateways 表结构更新 |
| `services/api-server/src/calls.rs` | route_preview 显示前缀处理 |
| `services/api-server/src/main.rs` | 网关 CRUD 支持新字段 |
| `web/src/types/index.ts` | SipGateway 类型更新 |
| `web/src/pages/Gateways/index.tsx` | 网关管理页面新增字段 |
| `web/src/pages/Routes/index.tsx` | 路由试算显示前缀处理 |

### 6. 验证

1. 数据库迁移成功
2. 创建带前缀处理的网关
3. 路由试算显示正确的前缀处理
4. 实际呼叫验证前缀处理生效
