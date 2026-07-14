export type ConfigValueType = 'boolean' | 'integer' | 'number' | 'string' | 'secret';

export interface ConfigField {
  key: string;
  label: string;
  description: string;
  type: ConfigValueType;
  min?: number;
  max?: number;
  step?: number;
  placeholder?: string;
}

export interface ConfigGroup {
  key: string;
  title: string;
  description: string;
  fields: ConfigField[];
}

export const CONFIG_GROUPS: ConfigGroup[] = [
  {
    key: 'sip',
    title: 'SIP 信令',
    description: '会话定时器和呼叫侧状态管理参数。',
    fields: [
      { key: 'session_expires_gateway', label: '网关会话过期时间', description: '网关中继方向 Session-Expires，单位秒。', type: 'integer', min: 60, max: 86400 },
      { key: 'session_expires_caller', label: '终端会话过期时间', description: '主叫终端方向 Session-Expires，单位秒。', type: 'integer', min: 60, max: 86400 },
    ],
  },
  {
    key: 'media',
    title: 'RTP 媒体',
    description: '媒体通告、端口分配、NAT 学习和安全策略。',
    fields: [
      { key: 'rtp_advertised_addr', label: 'RTP 通告地址', description: '写入 SDP 的媒体公网 IP，不包含端口。', type: 'string', placeholder: '例如 203.0.113.10' },
      { key: 'rtp_port_min', label: 'RTP 最小端口', description: 'RTP 端口池起点，建议使用偶数。', type: 'integer', min: 1024, max: 65534, step: 2 },
      { key: 'rtp_port_max', label: 'RTP 最大端口', description: 'RTP 端口池终点，必须大于最小端口。', type: 'integer', min: 1024, max: 65534, step: 2 },
      { key: 'rtp_symmetric_learning', label: '对称 RTP 学习', description: '根据收到的数据包学习远端媒体地址。', type: 'boolean' },
      { key: 'rtp_anti_spoofing', label: 'RTP 防欺骗', description: '拒绝不符合已学习源地址的媒体包。', type: 'boolean' },
      { key: 'rtp_source_relearn_secs', label: '源地址重新学习间隔', description: '允许重新学习 RTP 源地址的最短秒数。', type: 'integer', min: 1, max: 3600 },
      { key: 'media_metrics_log', label: '媒体指标明细日志', description: '通话结束时记录每路 RTP 指标，压测时建议关闭。', type: 'boolean' },
    ],
  },
  {
    key: 'recording',
    title: '录音',
    description: '录音开关、异步写入容量和磁盘保护参数。',
    fields: [
      { key: 'recording_enabled', label: '启用通话录音', description: '全局录音总开关。', type: 'boolean' },
      { key: 'recording_dir', label: '本地录音目录', description: 'sip-edge 进程可写的录音根目录。', type: 'string', placeholder: '/var/lib/vos-rs/recordings' },
      { key: 'recording_workers', label: '录音工作线程', description: '异步录音文件写入线程数量。', type: 'integer', min: 1, max: 64 },
      { key: 'recording_queue_capacity', label: '录音队列容量', description: '录音管道最大待处理消息数。', type: 'integer', min: 128, max: 1000000 },
      { key: 'recording_retention_secs', label: '录音保留时间', description: '本地录音保留秒数，0 表示不按时间清理。', type: 'integer', min: 0 },
      { key: 'recording_min_free_bytes', label: '磁盘最小剩余字节', description: '低于该空间时停止创建新录音。', type: 'integer', min: 0 },
      { key: 'recording_max_file_bytes', label: '单文件最大字节', description: '单个 WAV 文件上限，0 表示不限制。', type: 'integer', min: 0 },
      { key: 'recording_max_duration_secs', label: '单次录音最长时间', description: '单个录音最长秒数，0 表示不限制。', type: 'integer', min: 0 },
    ],
  },
  {
    key: 'performance',
    title: '性能与队列',
    description: 'UDP 收发和 CDR 内存队列参数。修改前建议先做压测。',
    fields: [
      { key: 'udp_workers_auto', label: '自动选择 UDP Worker', description: '根据 CPU 数量自动选择 UDP worker。', type: 'boolean' },
      { key: 'udp_workers', label: 'UDP Worker 数量', description: '关闭自动模式时使用的 worker 数量。', type: 'integer', min: 1, max: 256 },
      { key: 'udp_receive_buffer_bytes', label: 'UDP 接收缓冲区', description: 'Socket 接收缓冲区字节数。', type: 'integer', min: 65536, max: 1073741824 },
      { key: 'udp_send_buffer_bytes', label: 'UDP 发送缓冲区', description: 'Socket 发送缓冲区字节数。', type: 'integer', min: 65536, max: 1073741824 },
      { key: 'cdr_queue_capacity', label: 'CDR 内存队列容量', description: '数据库或 NATS 短暂故障时的有界缓冲容量。', type: 'integer', min: 128, max: 1000000 },
      { key: 'cdr_persistence_enabled', label: '启用 CDR 持久化', description: '关闭后仍维护呼叫状态，但不把 CDR 写入数据库或消息队列，仅建议隔离压测使用。', type: 'boolean' },
      { key: 'gateway_health_checks_enabled', label: '启用网关健康检查', description: '控制 OPTIONS 探测以及网关健康状态的加载与持久化。', type: 'boolean' },
    ],
  },
  {
    key: 'sbc',
    title: 'SBC 与认证',
    description: 'SIP 限速、并发控制和 Digest 认证参数。',
    fields: [
      { key: 'sbc_rate_limit_capacity', label: '令牌桶容量', description: '单 IP 突发请求容量。', type: 'number', min: 1, max: 10000000, step: 1 },
      { key: 'sbc_rate_limit_fill_rate', label: '令牌填充速率', description: '每秒补充的令牌数。', type: 'number', min: 0.1, max: 10000000, step: 0.1 },
      { key: 'sbc_max_concurrency', label: '最大并发通话', description: 'SBC 接受的并发通话上限。', type: 'integer', min: 1, max: 10000000 },
      { key: 'realm', label: 'Digest Realm', description: 'SIP Digest 挑战域。', type: 'string' },
      { key: 'nonce', label: 'Digest Nonce', description: '静态兼容 nonce；生产环境建议定期更换。', type: 'secret' },
      { key: 'secret_key', label: '认证签名密钥', description: '用于动态 nonce 签名，不会在界面中明文展示。', type: 'secret' },
    ],
  },
  {
    key: 'billing',
    title: '实时计费',
    description: '余额预检、余额耗尽强制拆线及通话结束结算策略。',
    fields: [
      { key: 'balance_enforcement_enabled', label: '启用余额强制策略', description: '呼叫前检查余额，并在余额耗尽时自动拆线。压测环境可关闭。', type: 'boolean' },
      { key: 'billing_settlement_enabled', label: '启用通话结束结算', description: '通话结束时计算费率并写入余额流水；隔离性能测试时可关闭。', type: 'boolean' },
    ],
  },
  {
    key: 'tls',
    title: 'SIP TLS',
    description: '证书校验和客户端 TLS 参数。跳过校验仅用于隔离测试环境。',
    fields: [
      { key: 'tls_bind_addr', label: 'TLS 监听地址', description: '例如 0.0.0.0:5061；留空表示不监听。', type: 'string' },
      { key: 'tls_cert_path', label: '证书路径', description: 'PEM 证书文件路径。', type: 'string' },
      { key: 'tls_key_path', label: '私钥路径', description: 'PEM 私钥文件路径。', type: 'string' },
      { key: 'tls_ca_path', label: 'CA 证书路径', description: '用于验证上游网关的 CA 文件。', type: 'string' },
      { key: 'tls_server_name', label: 'TLS Server Name', description: '连接上游时用于 SNI 和证书校验。', type: 'string' },
      { key: 'tls_allow_test_certificate', label: '允许测试证书', description: '接受项目测试证书。', type: 'boolean' },
      { key: 'tls_insecure_skip_verify', label: '跳过证书校验', description: '危险：仅限隔离测试环境。', type: 'boolean' },
    ],
  },
];

export const CONFIG_FIELDS = CONFIG_GROUPS.flatMap((group) => group.fields);
export const CONFIG_FIELD_MAP = new Map(CONFIG_FIELDS.map((field) => [field.key, field]));
