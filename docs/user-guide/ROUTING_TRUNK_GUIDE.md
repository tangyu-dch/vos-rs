# VOS-RS 中继与路由配置指南

本文说明管理控制台中的“中继管理”和“路由管理”如何配置，以及配置在
`api-server`、PostgreSQL 和 `sip-edge` 之间如何生效。内容以当前代码实现为准。

## 1. 核心概念

- **中继（Trunk/Gateway）**：外部 SIP 对端的地址与容量配置，存储在
  `sip_gateways`。
- **路由规则（Route）**：把被叫号码前缀映射到一个中继，存储在
  `sip_routes`。
- **候选路由**：同一号码可以匹配多条规则。SIP Edge 会生成有顺序的候选列表，
  首选失败时可切换到下一候选。
- **健康状态**：SIP Edge 使用 OPTIONS 探测和实际 INVITE 响应维护中继熔断状态，
  状态存储在 `gateway_health_status`。

建议始终先创建中继，再创建引用该中继的路由。删除中继时，数据库外键会级联删除
关联路由。

## 2. 配置生效链路

```text
控制台
  -> api-server REST API
  -> PostgreSQL（sip_gateways / sip_routes）
  -> 发布 NATS vos_rs.routing.reload
  -> sip-edge 重新读取启用的中继和全部路由
  -> 替换内存 RouteTable
```

当 NATS 未连接或发布失败时，API 写入仍可能成功。SIP Edge 在启用动态数据库路由时
每 60 秒周期刷新一次，因此配置会延迟生效。启动时也会从数据库加载一次。

相关系统配置：

```yaml
sip_edge:
  routing:
    default_gateway: ""
    database_routes_enabled: true
    gateway_health_checks_enabled: true
```

- `database_routes_enabled=true`：使用数据库中继和路由；否则使用
  `default_gateway` 生成一条空前缀默认路由。
- `gateway_health_checks_enabled=true`：启动 SIP OPTIONS 探测并持久化健康状态。
- `default_gateway`：数据库中没有中继时可在启动阶段创建 `default` 中继和默认路由。

项目启用了数据库动态配置时，上述开关还可由 `system_configs` 中的同名配置覆盖。

## 3. 中继字段

### 3.1 当前呼叫链路实际生效字段

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | 字符串 | 中继唯一 ID；路由通过此字段引用中继 |
| `host` | 字符串 | SIP 对端主机名或 IP |
| `port` | 1-65535 | SIP 对端端口；为空时出站默认使用 5060 |
| `transport` | `udp` | 当前 API 与管理页面只允许 UDP；TCP/TLS 出站尚未接通 |
| `prefix_rules` | 字符串/空 | 加载到运行时目标，在生成出站 URI 时按顺序改写被叫号码 |
| `max_capacity` | 正整数/空 | 中继总并发上限；为空表示不限，达到上限时跳过该中继；API 将 0 规范化为空 |
| `enabled` | 布尔值 | 只有启用的中继会被加载到 SIP Edge 路由表 |

当前数据库路由加载器会把 `transport` 带入 `RouteTarget`，但 SIP Edge 的出站
`PendingDatagram` 仍使用 UDP socket。为避免出现“保存成功但无法拨通”，API 会拒绝
`tcp`/`tls`，管理页面也只提供 UDP。待 TCP/TLS 发送链路完成端到端验证后再开放选项。

### 3.2 已存储但当前数据库呼叫链路尚未生效字段

| 字段 | 当前用途/限制 |
|---|---|
| `gateway_type` | 管理列表筛选和业务分类，不参与选路 |
| `supports_registration` | 配置元数据；当前中继出站链路未实现按此字段发起注册 |
| `reg_auth_type` | 配置元数据 |
| `reg_username` | 配置元数据 |
| `reg_password` | 配置元数据；更新请求省略时保留原密码 |
| `caller_id_mode` | 已加载到 `RouteTarget` 并随选路结果返回，但当前未找到出站 INVITE `From` 改写调用点 |
| `virtual_caller` | 已加载到 `RouteTarget`，但是否改写实际 `From` 与 `caller_id_mode` 有相同限制 |
| `max_concurrent` | API/数据库可保存；当前选路容量判断使用 `max_capacity` |
| `account_id` | 管理关联元数据，当前不参与路由选择 |

`parent_gateway_id`、`current_concurrent` 和 `circuit_state` 会出现在详情模型中，但当前
创建/编辑接口不允许直接设置父中继或健康运行态。运行态由 SIP Edge 维护。

### 3.3 中继配置步骤

1. 在“中继管理”点击“新建中继”。
2. 为 `id` 使用稳定且可识别的名称，例如 `cmcc-shanghai-a`。
3. 填写可从 SIP Edge 网络访问的 `host` 和 `port`。
4. 当前传输协议固定选择 `udp`；TCP/TLS 待出站传输链路接通后开放。
5. 按运营商容量设置 `max_capacity`，并启用中继。
6. 保存后打开中继详情，检查健康状态、当前通话、关联号码和依赖路由。
7. 再到“路由管理”为该中继创建路由。

## 4. 路由字段

| 字段 | 类型 | 生效方式 |
|---|---|---|
| `id` | 字符串 | 路由唯一 ID；编辑时 URL 中的 ID 为准 |
| `prefix` | 字符串 | 被叫号码前缀；空字符串是兜底路由 |
| `priority` | 0-65535 | 同前缀精度下数字越大越优先；API 会校验范围 |
| `gateway_id` | 字符串 | 目标中继 ID，必须存在 |
| `cost` | 大于等于 0 的有限数值 | 同前缀长度和优先级下越低越优先 |
| `weight` | 整数 | 同前缀长度、优先级、成本时做加权随机；API 限制为 1-10000 |
| `time_start` | `HH:MM` 字符串/空 | 与 `time_end` 同时存在时才执行时间过滤 |
| `time_end` | `HH:MM` 字符串/空 | 当前实现按字符串比较当前 UTC 时间的 `HH:MM` |

注意：当前控制台表单只展示 `id`、`prefix`、`priority`、`gateway_id`、`cost` 和
`weight`；时间窗可通过 API 配置。API 要求起止时间同时填写且格式为 `HH:MM`。
同日窗口使用闭区间判断；`22:00-06:00` 这类起始时间大于结束时间的窗口会按跨午夜
窗口处理。比较基准为 UTC 时间。

## 5. 实际选路顺序

SIP Edge 按以下顺序生成候选路由：

1. 被叫号码必须以规则 `prefix` 开头；空前缀可匹配所有号码。
2. **前缀越长越优先**，例如 `8613` 优先于 `86`。
3. 相同前缀长度时，**`priority` 越大越优先**。
4. 前两项相同时，**`cost` 越低越优先**。
5. 前三项完全相同时，按 `weight` 加权随机排列候选；权重越大越可能排在前面。
6. 排除已熔断或已达到 `max_capacity` 的中继。
7. 取第一个可用候选发起 INVITE，同时保留后续候选用于故障切换。

“最长前缀”比较的是前缀长度，不是字符串值。路由管理列表的数据库展示顺序
（`priority` 升序、`id`）不等于实际呼叫候选顺序。

## 6. 健康检查、容量与故障切换

### 6.1 OPTIONS 健康检查

启用健康检查后，SIP Edge 每 10 秒向所有启用中继发送一次 SIP OPTIONS；单次探测
3 秒未收到响应会记录失败日志。健康状态持久化到 `gateway_health_status`。

默认熔断参数由 `call-core` 固定：

- 连续失败 5 次进入 `Open`。
- `Open` 30 秒后允许进入 `HalfOpen` 探测。
- `HalfOpen` 连续成功 5 次恢复 `Closed`。
- 至少 10 个样本后，成功率低于 30% 也视为不可用。

实际 INVITE 的 2xx 响应计为成功，4xx 及以上响应计为失败。当前 OPTIONS 发送使用
UDP，且探测请求的目标用户为 `health-check`。

OPTIONS 非 2xx、发送失败和超时都会调用健康追踪器的 `record_failure`，增加连续失败
计数并持久化最新状态；OPTIONS 2xx 会调用 `record_success`。因此主动探测和实际 INVITE
结果都会推进中继熔断状态。

### 6.2 并发容量

选路时使用健康追踪器维护的活跃呼叫数与中继 `max_capacity` 比较：

```text
active_calls < max_capacity  -> 可选
active_calls >= max_capacity -> 跳过并尝试下一候选
```

发出中继 INVITE 时增加活跃数，通话结束、失败或切换中继时减少。管理详情中的
`current_concurrent` 来自持久化健康状态，而不是由表单手工维护。

### 6.3 故障切换

- 首选中继返回 `408` 或 `5xx`，并且还有下一候选时，自动向下一候选重新发 INVITE。
- 首选中继返回其他 `4xx` 或 `6xx` 时，当前实现结束呼叫，不自动切换。
- 客户端事务超时也可触发下一候选。
- 切换时会调整旧、新中继活跃数，并重新建立媒体转发目标。
- 请求带 `X-Forking-Enabled: true` 或 `X-Call-Forking: true` 时，最多并行尝试前三个候选；
  这不是控制台默认行为。

## 7. 号码改写现状

`call-core` 已实现中继前缀规则语法：

| 规则 | 含义 | 示例 |
|---|---|---|
| `abc:def` | 匹配并替换开头 | `86:0086` |
| `:def` | 无条件添加前缀 | `:9` |
| `abc:` | 匹配并删除开头 | `0086:` |
| 逗号分隔 | 按顺序执行多条 | `0:86,:9` |

数据库热加载现在会把 `prefix_rules` 写入运行时 `RouteTarget`，并在
`outbound_uri_for` 中改写被叫用户部分，因此该规则会作用于实际出站 Request-URI。

`caller_id_mode` 和 `virtual_caller` 也会进入 `RouteTarget` 并随选路结果返回，但当前代码
尚未找到把它们应用到出站 INVITE `From` 头的调用点。因此可依赖被叫前缀改写，主叫
号码改写仍需在抓包确认后使用。

## 8. 典型配置

### 8.1 主备中继

```text
中继：carrier-a，host=10.10.0.10，max_capacity=300
中继：carrier-b，host=10.10.0.11，max_capacity=200

路由：cn-a，prefix=86，priority=200，cost=0.05，weight=100，gateway_id=carrier-a
路由：cn-b，prefix=86，priority=100，cost=0.04，weight=100，gateway_id=carrier-b
```

虽然 `carrier-b` 成本更低，但 `priority` 先于成本比较，因此 `carrier-a` 是首选；
`carrier-a` 不健康、满载、返回 408/5xx 或事务超时时才尝试 `carrier-b`。

### 8.2 同等级加权分流

```text
route-a：prefix=86，priority=100，cost=0.05，weight=70，gateway_id=carrier-a
route-b：prefix=86，priority=100，cost=0.05，weight=30，gateway_id=carrier-b
```

两条规则的前三个排序条件完全相同时，候选首位大致按 70:30 分布。权重不是并发
上限，也不保证短时间窗口内严格达到比例。

### 8.3 最长前缀与默认路由

```text
mobile：prefix=8613，priority=100，cost=0.03，gateway_id=mobile-carrier
china： prefix=86，  priority=500，cost=0.02，gateway_id=china-carrier
default：prefix=，  priority=1000，cost=0.01，gateway_id=default-carrier
```

号码 `8613800138000` 首先匹配 `mobile`。即使更短规则的优先级更大或成本更低，
也不会越过更长前缀。

## 9. REST API

API 使用 JWT 鉴权。`admin` 可访问全部接口，`operator` 可管理中继和路由，
`financier` 不可修改这些运营配置。

### 9.1 中继

```http
GET    /api/v1/trunks?page=1&page_size=20&gateway_type=peer
POST   /api/v1/trunks
GET    /api/v1/trunks/:id
PUT    /api/v1/trunks/:id
DELETE /api/v1/trunks/:id
```

创建示例：

```json
{
  "id": "carrier-a",
  "host": "10.10.0.10",
  "port": 5060,
  "transport": "udp",
  "gateway_type": "peer",
  "max_capacity": 300,
  "enabled": true
}
```

更新请求要求提供 `host` 和 `transport`。密码字段省略时会保留原值；空字符串不是
“省略”。

### 9.2 路由

```http
GET    /api/v1/routing/rules?page=1&page_size=20
POST   /api/v1/routing/rules
PUT    /api/v1/routing/rules/:id
DELETE /api/v1/routing/rules/:id
GET    /api/v1/routing/simulations?destination=8613800138000
```

创建示例：

```json
{
  "id": "cn-mobile-primary",
  "prefix": "8613",
  "priority": 200,
  "gateway_id": "carrier-a",
  "cost": 0.05,
  "weight": 100,
  "time_start": "08:00",
  "time_end": "22:00"
}
```

更新时，URL 中的 `:id` 是最终路由 ID，请求体不包含 `id`。PUT 只更新已存在规则，
目标 ID 不存在时返回错误，不再隐式创建新规则。创建或修改后 API 会尝试发布路由
重载通知。

路由仿真由 API Server 转发到 SIP Edge `/manage/route-preview`，返回当前内存路由表的
候选顺序。它使用当前已经加载且经过时间窗过滤的路由，但预览本身不执行健康状态和
容量过滤，因此最终实呼叫可能跳过预览中的某个中继。

## 10. 排障清单

### 路由保存了但呼叫没有使用

1. 确认 `database_routes_enabled=true`。
2. 确认路由引用的中继存在且 `enabled=true`。
3. 确认被叫号码确实以 `prefix` 开头；兜底规则使用空前缀。
4. 检查 UTC 时间窗；两个端点必须同时配置，跨午夜窗口可直接使用 `22:00-06:00`。
5. 执行路由仿真，确认规则已进入 SIP Edge 内存表。
6. 查看 API/SIP Edge 日志中 NATS 重载失败提示；失败时等待最多 60 秒周期刷新。

### 中继被跳过

1. 在中继详情检查 `enabled`、`health.state`、`active_calls` 和 `capacity`。
2. `state=open` 表示熔断；检查 OPTIONS 是否能在 3 秒内收到响应。
3. 确认防火墙允许 SIP Edge 到中继的 UDP 端口及返回流量。
4. 检查 `active_calls` 是否达到 `max_capacity`。
5. 检查中继是否实际返回大量 4xx/5xx；4xx 也会计入健康失败样本。

### 仿真有候选但实呼失败

仿真不检查健康和容量。继续检查中继状态，并抓取 SIP 响应码。只有 408、5xx 或事务
超时会自动顺序切换；普通 4xx 不会切换。

### 配置了号码改写但未生效

先用路由仿真确认目标中继，再抓取出站 INVITE。`prefix_rules` 只改写 Request-URI 的
被叫用户部分，不改写 `To` 或主叫 `From`。主叫策略虽已进入运行时目标，但当前仍未
确认应用到 `From`，不要把 `caller_id_mode` 保存成功等同于主叫改写已经生效。
