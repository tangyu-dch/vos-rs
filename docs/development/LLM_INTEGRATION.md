# 🤖 vos-rs 大模型 (LLM) 配置文件修改与对接集成指南

本文档介绍如何在 **vos-rs** 电信软交换与智能媒体平台中通过**修改本地 YAML 配置文件 (`config/vos_config.yaml`)** 配置与对接各类大语言模型（LLM）、语音大模型（Realtime Audio Models）以及自建 AI 模型服务。

---

## 📖 目录

1. [配置文件修改入口 (推荐方式)](#1-配置文件修改入口-推荐方式)
2. [支持的大模型 YAML 配置示例](#2-支持的大模型-yaml-配置示例)
   - [OpenAI (GPT-4o Realtime & GPT-4o)](#21-openai-gpt-4o-realtime--gpt-4o)
   - [Google Gemini Live / Gemini 1.5 Pro](#22-google-gemini-live--gemini-15-pro)
   - [DeepSeek (V3 / R1)](#23-deepseek-v3--r1)
   - [本地自建 vLLM / Ollama](#24-本地自建-vllm--ollama)
3. [Web 控制台在线配置修改](#3-web-控制台在线配置修改)
4. [代码层可编程 Audio Token 钩子 API](#4-代码层可编程-audio-token-钩子-api)

---

## 1. 配置文件修改入口 (推荐方式)

**vos-rs** 支持通过项目根目录下的全局配置文件 **`config/vos_config.yaml`** 进行集中式的 LLM 大模型与 AI Voice 凭证管理。

配置文件路径：**`config/vos_config.yaml`**

### 标准 YAML 配置结构：

```yaml
# ===========================================================================
#                      VOS-RS 全局系统与 LLM 核心配置文件
#                            config/vos_config.yaml
# ===========================================================================

server:
  host: "0.0.0.0"
  port: 8081

# ---------------------------------------------------------------------------
# 🤖 大模型 (LLM) 与 AI Voice Agent 核心对接配置
# ---------------------------------------------------------------------------
llm_integration:
  # 是否启用 LLM 集成 (true / false)
  enabled: true

  # 模型提供商: "openai" | "gemini" | "deepseek" | "local_vllm" | "ollama"
  provider: "openai"

  # 大模型 API 密钥 (API Key)
  api_key: "sk-proj-your-actual-api-key-here"

  # 大模型 REST 基础 Endpoint 地址
  base_url: "https://api.openai.com/v1"

  # 默认模型名称
  model: "gpt-4o-realtime-preview"

  # OpenAI Realtime 全双工音频 Token 直连 WebSocket 地址 (选填)
  realtime_websocket_url: "wss://api.openai.com/v1/realtime?model=gpt-4o-realtime-preview-2024-10-01"

  # 采样温度 (0.0 ~ 1.0)
  temperature: 0.7

  # 音频打断 (Barge-in) 静音检测敏感度 (0.0 ~ 1.0)
  vad_threshold: 0.65
```

修改该文件后，重启 `api-server` 服务或在 Web 控制台点击刷新，系统将自动读取最新的大模型对接凭证。

---

## 2. 支持的大模型 YAML 配置示例

### 2.1 OpenAI (GPT-4o Realtime & GPT-4o)

打开 `config/vos_config.yaml` 并修改：

```yaml
llm_integration:
  enabled: true
  provider: "openai"
  api_key: "sk-proj-xxxxxxxxxxxxxxxxxxxxxxxx"
  base_url: "https://api.openai.com/v1"
  model: "gpt-4o-realtime-preview"
  realtime_websocket_url: "wss://api.openai.com/v1/realtime?model=gpt-4o-realtime-preview-2024-10-01"
  temperature: 0.7
```

### 2.2 Google Gemini Live / Gemini 1.5 Pro

修改 `config/vos_config.yaml`：

```yaml
llm_integration:
  enabled: true
  provider: "gemini"
  api_key: "AIzaSyYourGeminiApiKeyHere"
  base_url: "https://generativelanguage.googleapis.com/v1beta"
  model: "gemini-1.5-pro"
  temperature: 0.2
```

### 2.3 DeepSeek (V3 / R1)

修改 `config/vos_config.yaml`：

```yaml
llm_integration:
  enabled: true
  provider: "deepseek"
  api_key: "sk-deepseek-your-api-key-here"
  base_url: "https://api.deepseek.com/v1"
  model: "deepseek-chat"
  temperature: 0.3
```

### 2.4 本地自建 vLLM / Ollama

针对私有化部署或内网无公网连接的环境，修改 `config/vos_config.yaml`：

```yaml
# Ollama 示例
llm_integration:
  enabled: true
  provider: "ollama"
  api_key: "not-needed"
  base_url: "http://localhost:11434/v1"
  model: "qwen2.5-coder:7b"

# 自建 vLLM 示例
# base_url: "http://192.168.1.100:8000/v1"
# model: "qwen2.5-72b-instruct"
```

---

## 3. Web 控制台在线配置修改

如果您不想手动编辑磁盘文件，也可以直接在图形化控制台中在线修改：

1. 打开浏览器登录 Web 控制台：**`http://localhost:3001/#/settings`**（或点击左侧导航 **`[系统与安全] -> [系统设置]`**）；
2. 找到 **`[大模型与 AI Voice 配置]`** 面板；
3. 在表单中填入您的 **LLM Endpoint**、**API Key** 与 **默认 Model**；
4. 点击 **`[保存配置]`**，修改会自动写入配置文件并即时更新生效。

---

## 4. 代码层可编程 Audio Token 钩子 API

对于二次开发，`services/media-edge` 提供了音频 Token 流处理钩子 `ai_plugin.rs`：

```rust
use media_edge::ai_plugin::AudioTokenHook;

pub struct CustomLlmConnector;

impl AudioTokenHook for CustomLlmConnector {
    fn on_pcm_frame_received(&self, call_id: &str, pcm_data: &[i16]) {
        // 1. 收到解包后的 RTP PCM 音频采样
        // 2. 打包为 WebSocket Audio Token 发送给在 config/vos_config.yaml 中配置的模型 Endpoint
    }

    fn on_llm_audio_token_reply(&self, call_id: &str, output_pcm: &[i16]) {
        // 1. 收到 LLM 返回的语音 Token
        // 2. 注入 RTP Jitter Buffer 播报给通话主叫方
    }
}
```
