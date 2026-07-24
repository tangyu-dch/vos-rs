# sip-core

电信级 SIP 2.0 (RFC 3261) 协议解析与构造内核。

## 设计特性与理念

- ⚡ **零外部依赖 (Zero External Dependencies)**：故意保持 `0` 外部依赖，完全基于 Rust 标准库实现，轻量、稳定且无供应链风险。
- 🚀 **零拷贝解析 (Zero-Copy Parsing)**：支持生命周期绑定的 `SipMessageBorrow<'a>`，直接在 UDP/TCP 字节 Buffer 切片上完成 SIP 报文解析，单包解析时延 `<50ns`。
- 🛡️ **内存与安全防护**：防范超长 Header、畸形 URI、格式化攻击，全可恢复错误显式返回 `SipResult<T>`。
- 📦 **协议覆盖度**：完整覆盖 `INVITE`, `ACK`, `BYE`, `CANCEL`, `REGISTER`, `OPTIONS`, `INFO`, `NOTIFY`, `SUBSCRIBE` 等常用 SIP 方法与核心 Header。
