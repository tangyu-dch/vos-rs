# VOS-RS 标准 AI 语音插件协议对接与开发指南 (AI Voice Plugin Protocol)

本指南旨在向第三方 AI 开发人员说明如何使用任意编程语言（如 Python、Go、Node.js 等）开发外部 AI 语音插件，与 VOS-RS 媒体面（`media-edge`）进行低延迟的双向流式语音交互。

---

## 1. 协议核心规范

VOS-RS 与外部 AI 语音插件之间的数据交互默认采用低延迟的 **UDP 套接字或 WebSocket 二进制帧** 传输。双方收发的音频数据包均遵循以下标准二进制帧结构，每次传输大小固定为 **336 字节**：

### 1.1 二进制帧格式 (Total: 336 字节)

| 字节偏移 (Bytes) | 字段名 (Field) | 类型 (Type) | 字节序 (Endian) | 说明 (Description) |
|:---|:---|:---|:---|:---|
| `0..4` (4字节) | **CallID** | `uint32` | Big-Endian (网络字节序) | 当前通话会话的唯一标识 ID |
| `4..8` (4字节) | **Seq** | `uint32` | Big-Endian (网络字节序) | 音频帧的自增序列号，用于防丢包与抖动重排 |
| `8..16` (8字节) | **Timestamp** | `uint64` | Big-Endian (网络字节序) | 音频产生的时间戳（毫秒级） |
| `16..336` (320字节) | **Payload** | `[u8; 320]` | 原始 PCM16 | 20ms 的 8000Hz 采样率、单声道、16-bit PCM 音频载荷 |

---

## 2. 外部 AI 插件开发示例

### 2.1 Python 对接实现示例 (AI 生态首选)

以下是使用 Python 异步 IO（`asyncio`）编写的简易 AI 插件接收与回应服务：

```python
import asyncio
import struct

AI_HEADER_SIZE = 16
AI_PAYLOAD_SIZE = 320
AI_TOTAL_SIZE = AI_HEADER_SIZE + AI_PAYLOAD_SIZE

class AiVoicePluginServer:
    def __init__(self, host="127.0.0.1", port=23456):
        self.host = host
        self.port = port

    def connection_made(self, transport):
        self.transport = transport
        print(f"AI Plugin Server started on {self.host}:{self.port}")

    def datagram_received(self, data, addr):
        if len(data) < AI_TOTAL_SIZE:
            return

        # 1. 反序列化解析 VOS-RS 上行音频包
        call_id, seq, timestamp = struct.unpack("!IIQ", data[0:16])
        pcm_payload = data[16:AI_TOTAL_SIZE]
        
        # [业务逻辑点]：在这里把 pcm_payload 丢入您的 ASR (语音识别) -> LLM (大模型回复) -> TTS (语音合成)
        # print(f"Received from Call {call_id}: seq={seq}, ts={timestamp}, pcm_len={len(pcm_payload)}")

        # 2. 模拟 AI 实时生成 TTS 回复 (这里用 0x77 静音包作为测试回送)
        tts_pcm = b"\x77" * AI_PAYLOAD_SIZE
        response_seq = seq + 1
        response_ts = timestamp + 20

        # 3. 封装为标准二进制帧回送给 VOS-RS 下行
        response_packet = struct.pack("!IIQ", call_id, response_seq, response_ts) + tts_pcm
        self.transport.sendto(response_packet, addr)

async def main():
    loop = asyncio.get_running_loop()
    transport, protocol = await loop.create_datagram_endpoint(
        lambda: AiVoicePluginServer(),
        local_addr=("127.0.0.1", 23456)
    )
    try:
        await asyncio.sleep(3600)  # 挂起运行 1 小时
    finally:
        transport.close()

if __name__ == "__main__":
    asyncio.run(main())
```

---

### 2.2 Go 语言对接实现示例 (Go 高性能系统对接)

使用 Go 语言的高效原生 UDP 连接处理示范：

```go
package main

import (
	"encoding/binary"
	"fmt"
	"net"
)

const (
	AiHeaderSize  = 16
	AiPayloadSize = 320
	AiTotalSize   = 336
)

type AiFrame struct {
	CallID    uint32
	Seq       uint32
	Timestamp uint64
	PcmData   []byte
}

func main() {
	addr, err := net.ResolveUDPAddr("udp", "127.0.0.1:23456")
	if err != nil {
		panic(err)
	}

	conn, err := net.ListenUDP("udp", addr)
	if err != nil {
		panic(err)
	}
	defer conn.Close()
	fmt.Printf("Go AI Voice Plugin listening on %s\n", addr.String())

	buf := make([]byte, 1024)
	for {
		n, remoteAddr, err := conn.ReadFromUDP(buf)
		if err != nil {
			continue
		}
		if n < AiTotalSize {
			continue
		}

		// 1. 反序列化
		callID := binary.BigEndian.Uint32(buf[0:4])
		seq := binary.BigEndian.Uint32(buf[4:8])
		ts := binary.BigEndian.Uint64(buf[8:16])
		pcmPayload := buf[16:AiTotalSize]

		_ = pcmPayload // ASR/LLM/TTS 处理入口

		// 2. 回送下行 TTS
		respPcm := make([]byte, AiPayloadSize)
		for i := range respPcm {
			respPcm[i] = 0x55 // 填充模拟语音
		}

		respBuf := make([]byte, AiTotalSize)
		binary.BigEndian.PutUint32(respBuf[0:4], callID)
		binary.BigEndian.PutUint32(respBuf[4:8], seq+1)
		binary.BigEndian.PutUint64(respBuf[8:16], ts+20)
		copy(respBuf[16:], respPcm)

		conn.WriteToUDP(respBuf, remoteAddr)
	}
}
```

---

## 3. 联调与测试流程

当您启动了上述外部 AI 插件（例如 Python 服务，端口为 `23456`），VOS-RS 的媒体代理服务会运行 `AiVoicePluginProxy::start(proxy_addr, plugin_addr)`。
1. `media-edge` 在收到浏览器的音频包后，会自动解码并打包发送给 `127.0.0.1:23456`。
2. 您的 AI 插件在处理完 ASR+LLM+TTS 后，只需将回复的 336 字节音频帧直接 `sendto` 发送回来源的 `from` 地址即可。
3. `media-edge` 会将回传的音频自动送入主中继流程，网页浏览器即可实时听到大模型的语音播报，达成完整的低延迟闭环。
