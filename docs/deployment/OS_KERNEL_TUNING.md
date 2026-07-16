# 电信级 VoIP 软交换操作系统内核调优指南

为支持单机 **5000+ 并发媒体中继**、**1000+ CPS** 爆发打入以及每秒上百万数据包的极限吞吐，除了在应用层使用无锁原子位图与同步非阻塞 I/O 之外，必须从**操作系统内核 (Kernel)** 层面解除网络协议栈与文件系统的性能锁。

以下是针对 macOS（开发/压测环境）以及 Linux（生产环境）的电信级系统内核调优指南。

---

## 🍏 macOS 宿主机调优配置 (开发与压测环境)

macOS 默认内核参数面向桌面应用，在高频 UDP 数据流（如本地 SIPp 压测）下极易导致内核静默丢包。请在宿主机终端执行以下配置。

### 1. UDP/TCP 缓冲区深度扩容
```bash
# 1. 调大系统单套接字最大缓冲区限制 (默认 256KB -> 16MB)
sudo sysctl -w kern.ipc.maxsockbuf=16777216

# 2. 调大 UDP 默认接收缓冲区 (默认 42KB -> 8MB)
sudo sysctl -w net.inet.udp.recvspace=8388608

# 3. 调大 UDP 最大数据报文限制 (默认 9KB -> 64KB，防止大 SDP 包截断)
sudo sysctl -w net.inet.udp.maxdgram=65535

# 4. 调大 TCP 监听队列积压上限 (从默认 128 提升至 2048，防止高频 TCP/TLS 握手溢出)
sudo sysctl -w kern.ipc.somaxconn=2048
```

### 2. 进程最大打开文件描述符 (FD) 限制放开
macOS 默认限制了单进程文件描述符硬上限（通常为 10240）。高并发下这会导致分配媒体端口时触发 `EMFILE` 错误。
```bash
# 1. 调大系统总文件句柄限制
sudo sysctl -w kern.maxfiles=524288
sudo sysctl -w kern.maxfilesperproc=262144

# 2. 调整当前 shell 会话硬限制
ulimit -n 65535
```

---

## 🐧 Linux 宿主机调优配置 (生产运行环境)

在 Linux 生产级物理服务器运行 VOS-rs 时，请将以下参数写入配置文件 `/etc/sysctl.conf` 并执行 `sudo sysctl -p` 生效。

### 1. 核心网络栈与 UDP 深度扩容
在媒体中继模式下，单进程需要绑定两万多个 UDP 套接字，必须从内核级扩充缓冲队列。
```ini
# 调大系统全局最大接收/发送套接字缓冲区 (16MB)
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216

# 调大系统套接字默认接收/发送缓冲区 (2MB)
net.core.rmem_default = 2097152
net.core.wmem_default = 2097152

# 调整 UDP 内存页大小限制 (分段水位：min, pressure, max)
net.ipv4.udp_mem = 65536 131072 262144

# 扩大 IP 分片高/低内存占用限制 (针对大 SDP UDP 分包)
net.ipv4.ipfrag_high_thresh = 4194304
net.ipv4.ipfrag_low_thresh = 3145728
```

### 2. 网卡排队队列（Ring Buffer & Backlog）扩容
在高频软中断处理中，网口接收积压队列（backlog）偏低会导致数据包来不及被应用层 poll 读取而在 IP 层直接丢弃。
```ini
# 调大网络设备接收积压队列长度 (默认 1000 -> 20000)
net.core.netdev_max_backlog = 20000

# 增加每次 softirq 允许消费的数据包数量上限 (默认 300 -> 600)
net.core.netdev_budget = 600

# 禁用网卡接收包自动裁减（由物理网卡分配软中断）
net.core.optmem_max = 2048576
```

### 3. 防火墙连接跟踪表 (`nf_conntrack`) 网络风暴防护
当每秒有上百万个 UDP 包通过系统时，Linux 内核的 `nf_conntrack` 防火墙跟踪模块会迅速饱和，抛出 table full 丢包异常。
```ini
# 调大连接跟踪表最大容量
net.netfilter.nf_conntrack_max = 1048576

# 缩短 UDP 跟踪连接超时时间 (避免僵尸通道长时间占用跟踪表)
net.netfilter.nf_conntrack_timeouts_udp = 10
net.netfilter.nf_conntrack_timeouts_udp_stream = 30
```
> **最佳实践提示**：在生产服务器中，强烈建议直接在 `iptables` 中配置 `NOTRACK` 规则，绕过防火墙对 RTP 媒体端口范围（如 10000-40000）的追踪：
> ```bash
> iptables -t raw -A PREROUTING -p udp --dport 10000:40000 -j NOTRACK
> ```

### 4. 文件描述符限制
```ini
fs.file-max = 2097152
fs.nr_open = 1048576
```
请同时配置 `/etc/security/limits.conf` file，为运行软交换的用户分配硬限制：
```text
* soft nofile 1048576
* hard nofile 1048576
```
