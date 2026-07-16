# VOS-RS Web 控制台与管理 API 开发指引

本指引涵盖 `vos-rs` 管理后台的各个功能模块、前端 React（Vite + TS + Arco Design）项目架构以及后台 REST API 与可编程媒体 API 接口细节。

中继字段、路由排序、健康检查、故障切换和配置示例详见
[《VOS-RS 中继与路由配置指南》](./ROUTING_TRUNK_GUIDE.md)。

接入中继、IP/注册认证、主叫号码池、号码唯一归属、分机外呼和目标落地改造的
目标方案详见
[《VOS-RS 中继接入、主叫号码与落地设计》](../architecture/TRUNK_CALLER_TERMINATION_DESIGN.md)。
前一份指南描述当前实现，本设计文档描述待分阶段落地的目标模型。

---

## 一、系统构成与开发路径

```text
vos-rs/
├── web/                       # 前端项目 (React 18 + TS + Arco Design + ECharts)
│   ├── src/
│   │   ├── pages/             # 页面组件（仪表盘、话单、路由、计费、录音等）
│   │   ├── services/          # 后端 REST API 请求层
│   │   └── components/        # 公用 UI 组件
│
├── services/api-server/       # Axum REST API 服务器 (提供给 Web 控制台的 HTTP 服务)
└── services/sip-edge/         # SIP 核心网关 (内置可编程媒体中继控制 API)
```

---

## 二、前端开发快速开始

### 1. 前置条件
确保安装了 `Node.js` (版本 `>= 18.0`)。

### 2. 启动开发服务器
```bash
cd web

# 1. 安装项目依赖
npm install

# 2. 启动本地开发服务，默认在 http://localhost:3000 启动
# 该端口已预配置了向后端 api-server (默认 8080 端口) 的 /api 代理
npm run dev
```

### 3. 构建与部署
```bash
# 静态构建，输出内容存放在 dist/ 目录下，常用于生产环境 Nginx 托管
npm run build
```

---

## 三、管理后端 API 接口参考 (api-server)

`api-server` 基于 `Axum` + `sqlx` 提供业务层 REST API 访问：

### 1. 仪表盘统计
*   **端点**: `GET /api/v1/dashboard/stats`
*   **作用**: 查询系统整体运行健康度及当日话单统计。
*   **响应示例**:
    ```json
    {
      "active_calls": 12,
      "today_total_calls": 1250,
      "today_answered_calls": 1050,
      "answer_rate": 84.0,
      "avg_mos": 4.12,
      "avg_loss_rate": 0.05,
      "avg_jitter_ms": 3.2,
      "registered_users": 45,
      "active_gateways": 3
    }
    ```

### 2. 呼叫话单查询 (CDR)
*   **端点**: `GET /api/v1/cdrs?page=1&page_size=20&caller=1001&callee=138&status=answered`
*   **作用**: 分页过滤查询历史 CDR。
*   **详情接口**: `GET /api/v1/cdrs/:call_id`
*   **DTMF 查询**: `GET /api/v1/cdrs/:call_id/dtmf` — 查询特定通话中识别到的带内带外 DTMF 按键序列。

### 3. SIP 分机账号配置 (CRUD)
*   **获取列表**: `GET /api/v1/users`
*   **创建/修改用户**: `POST /api/v1/users`
    *   **参数**: `{"username": "1002", "password": "secure_password"}`

### 4. 出站网关与时间路由管理
*   **中继网关 CRUD**: `GET /api/v1/gateways`、`POST /api/v1/gateways`
*   **路由规则 CRUD**: `GET /api/v1/routes`、`POST /api/v1/routes`
    *   **参数**: 支持 `time_start` 与 `time_end` 控制时间选路逻辑。

---

## 四、核心可编程媒体控制 API (sip-edge 内部暴露)

`sip-edge` 提供用于语音机器人 (AI Agent) 和可编程通信的热插拔媒体控制，供控制层/业务服务器直接调用（默认监听 `8082` 端口，需身份验证）：

### 1. 投递音频播放 (Play)
```http
POST /manage/calls/:call_id/play
Content-Type: application/json
Authorization: Bearer <internal_auth_token>

{
  "leg": "caller",
  "file_path": "/var/media/audio_16k.wav",
  "mode": "exclusive",
  "loop_playback": false
}
```
*   `leg`: `caller`（主叫听到）、`callee`（被叫听到）、`both`（双向）。
*   `mode`: `exclusive`（独占播放，另一侧语音在播放期间被彻底阻断）；`background`（背景混音中继）。
*   **自动重采样引擎**：文件可为 8000Hz、16000Hz (16kHz)、44100Hz、48000Hz 等任意采样率，系统线性插值模块会自动降采样至 8000Hz 发送，防止音速失常。

### 2. 停止音频播放 (Stop Play)
```http
POST /manage/calls/:call_id/stop-play
Content-Type: application/json

{
  "leg": "caller"
}
```

### 3. 通话静音拦截 (Mute) / 恢复 (Unmute)
```http
POST /manage/calls/:call_id/mute
Content-Type: application/json

{
  "leg": "caller"
}
```
*   静音后，该 Leg 的原始 RTP 数据包在进入网关中继环时会被直接拦截丢弃，不转发至对端，实现静音。

### 4. 获取通话实时媒体状态 (Status)
```http
GET /manage/calls/:call_id/status
```
*   **返回参数**:
    ```json
    {
      "call_id": "invite-sdp-offer@example.com",
      "caller": {
        "muted": false,
        "playback": {
          "file_path": "/var/media/audio_16k.wav",
          "mode": "exclusive",
          "loop_playback": false,
          "progress_percentage": 42.5
        }
      },
      "callee": {
        "muted": false,
        "playback": null
      }
    }
    ```

---

## 五、开发者故障排查指南

1.  **数据库连接出错**
    确保启动各个二进制前，正确配置了环境变量：
    `export DATABASE_URL=postgres://tangyu@127.0.0.1:5432/vos_rs`

2.  **CORS 跨域访问阻断**
    `api-server` 默认搭载了 `CorsLayer`（开发状态允许所有源访问）。若在生产中修改，请确保将管理后台的外部域名或 IP 加入 Allow 列表中。

3.  **媒体流播放无声音或丢包**
    - 检查本地音频文件路径是否存在，且网关进程是否有读权限。
    - 检查首包 Marker Bit 接收情况。硬终端常依靠 Marker Bit 识别 SSRC 的流重置。
    - 对独占播放结束后流的 SSRC 序列号/时间戳平滑过渡参数进行抓包分析，确保没有空洞序列发生。
