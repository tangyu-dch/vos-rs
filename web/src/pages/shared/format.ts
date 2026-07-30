// 通话详情/资源字段的展示格式化工具
// 从 console.tsx 拆分

import type { Entity } from '@/services/resources';

const valueText = (value: unknown) =>
  value === null || value === undefined || value === '' ? '—' : String(value);

const moneyFields = new Set([
  'balance',
  'credit_limit',
  'price_per_interval',
  'amount',
  'balance_after',
  'cost',
  'access_amount',
  'egress_cost',
  'balance_before',
]);

const moneyText = (value: unknown) => {
  if (value === null || value === undefined || value === '') return '—';
  const amount = Number(value);
  if (!Number.isFinite(amount)) return String(value);
  return amount.toLocaleString('zh-CN', { minimumFractionDigits: 0, maximumFractionDigits: 3 });
};

const durationSecondsText = (value: unknown) => {
  if (value === null || value === undefined || value === '') return '—';
  const milliseconds = Number(value);
  if (!Number.isFinite(milliseconds)) return String(value);
  return Math.ceil(milliseconds / 1000).toLocaleString('zh-CN', {
    minimumFractionDigits: 0,
    maximumFractionDigits: 0,
  });
};

/// 格式化 ISO 8601 / RFC 3339 字符串或时间戳为本地可读时间。
/// 后端 `time::serde::rfc3339` 输出形如 "2026-07-28T10:30:00Z"。
const datetimeText = (value: unknown) => {
  if (value === null || value === undefined || value === '') return '—';
  const numericValue = typeof value === 'number' ? value : Number.NaN;
  const date = new Date(Number.isFinite(numericValue) ? numericValue : String(value));
  if (!Number.isNaN(date.getTime())) {
    return date.toLocaleString('zh-CN', { hour12: false });
  }
  return String(value);
};

const callDetailLabels: Record<string, string> = {
  call_id: '通话 ID',
  caller: '主叫号码',
  callee: '被叫号码',
  started_at_ms: '开始时间',
  ringing_at_ms: '振铃时间',
  answered_at_ms: '接通时间',
  ended_at_ms: '结束时间',
  duration_ms: '通话时长',
  billable_duration_ms: '计费时长',
  ringing_duration_ms: '振铃时长',
  access_billed_duration_ms: '对接计费时长',
  access_amount: '对接费用',
  egress_billed_duration_ms: '落地计费时长',
  egress_cost: '成本费用',
  status: '通话状态',
  failure_status_code: '失败状态码',
  failure_reason: '失败原因',
  caller_rtcp_loss_rate: '主叫丢包率',
  caller_rtcp_jitter_ms: '主叫抖动',
  caller_rtcp_rtt_ms: '主叫往返时延',
  gateway_rtcp_loss_rate: '落地丢包率',
  gateway_rtcp_jitter_ms: '落地抖动',
  gateway_rtcp_rtt_ms: '落地往返时延',
  mos: '通话质量 MOS',
  dtmf_digits: '按键记录',
  recording_path: '录音文件',
  direction: '呼叫方向',
  state: '实时状态',
  gateway: '当前中继',
  muted: '静音状态',
  playback: '放音状态',
  file_path: '音频文件',
  mode: '播放模式',
  loop_playback: '循环播放',
  progress_percentage: '播放进度',
  runtime_availability: '实时状态',
  digit: '按键',
  source: '事件来源',
  timestamp_ms: '发生时间',
  rtp_timestamp: 'RTP 时间戳',
  volume: '音量',
  inserted_at: '写入时间',
  id: '资源标识',
  name: '资源名称',
  username: '用户账号',
  created_at: '创建时间',
  updated_at: '更新时间',
  enabled: '启用状态',
  host: '主机地址',
  port: '服务端口',
  transport: '传输协议',
  role: '资源类型',
  description: '说明',
  max_capacity: '容量上限',
  current_concurrent: '当前并发',
  max_concurrent: '最大并发',
  number: '号码',
};

const callValueLabels: Record<string, string> = {
  answered: '已接通',
  canceled: '已取消',
  failed: '失败',
  inbound: '呼入',
  outbound: '呼出',
  trunk: '接入中继',
  extension: '分机号码',
  passthrough: '透传主叫',
  strict_passthrough: '严格透传',
  fixed: '固定主叫',
  fixed_number: '固定号码',
  virtual_pool: '号码池主叫',
  random: '均匀随机',
  weighted_random: '权重随机',
  round_robin: '顺序轮询',
  stable_hash: '稳定哈希',
  available: '实时可用',
  not_active: '通话已结束',
  unavailable: '控制面不可用',
  access: '对接账户',
  egress: '落地账户',
  call_charge: '通话扣费',
  call_cost: '落地成本',
  credit: '账户充值',
  adjustment: '余额调整',
  reversal: '费用冲正',
  rtp: 'RTP 事件',
  'sip-info': 'SIP INFO',
};

const hangupCauseMap: Record<string, string> = {
  // Q.850 / Standard SIP Status Code Hangup Causes
  '16': 'NORMAL_CLEARING (正常挂机 / 主被叫主动挂机)',
  '17': 'USER_BUSY (用户忙 / 对方拒接或占线)',
  '18': 'NO_USER_RESPONSE (无响应 / 对方超时未接听)',
  '19': 'NO_ANSWER (无应答 / 振铃超时)',
  '20': 'SUBSCRIBER_ABSENT (用户不在 / 离线不可达)',
  '21': 'CALL_REJECTED (呼叫被拒绝 / 拒接)',
  '27': 'DESTINATION_OUT_OF_ORDER (目标故障 / 线路异常)',
  '28': 'INVALID_NUMBER_FORMAT (空号 / 无效号码格式)',
  '31': 'NORMAL_UNSPECIFIED (普通未指定)',
  '34': 'CIRCUIT_CONGESTION (线路拥塞 / 中继满载)',
  '38': 'NETWORK_OUT_OF_ORDER (网络故障)',
  '41': 'TEMPORARY_FAILURE (临时故障 / 服务不可用)',
  '42': 'SWITCH_CONGESTION (交换机拥塞)',
  '57': 'BEARERCAPABILITY_NOTAUTH (未授权 / 认证失败或鉴权拒绝)',
  '127': 'INTERWORKING (互通故障)',

  // SIP Specific Status Mappings
  '200': 'NORMAL_CLEARING (正常挂机 [SIP 200 OK])',
  '400': 'INVALID_NUMBER_FORMAT (错误请求 [SIP 400 Bad Request])',
  '401': 'BEARERCAPABILITY_NOTAUTH (未授权 [SIP 401 Unauthorized])',
  '403': 'CALL_REJECTED (禁止呼叫 [SIP 403 Forbidden])',
  '404': 'UNALLOCATED_NUMBER (空号 / 未找到号码 [SIP 404 Not Found])',
  '408': 'RECOVERY_ON_TIMER_EXPIRY (请求超时 [SIP 408 Request Timeout])',
  '480': 'SUBSCRIBER_ABSENT (用户不可达 [SIP 480 Temporarily Unavailable])',
  '486': 'USER_BUSY (线路忙 [SIP 486 Busy Here])',
  '487': 'ORIGINATOR_CANCEL (主叫取消 [SIP 487 Request Terminated])',
  '488': 'INCOMPATIBLE_DESTINATION (媒体协商失败 / Codec 不匹配 [SIP 488 Not Acceptable])',
  '500': 'NETWORK_OUT_OF_ORDER (服务器内部错误 [SIP 500 Server Error])',
  '502': 'NETWORK_OUT_OF_ORDER (网关错误 [SIP 502 Bad Gateway])',
  '503': 'CIRCUIT_CONGESTION (服务不可用 / 网关满载 [SIP 503 Service Unavailable])',
  '504': 'RECOVERY_ON_TIMER_EXPIRY (网关超时 [SIP 504 Gateway Timeout])',
};

export function hangupCauseText(
  code: number | null | undefined,
  reason: string | null | undefined,
): string {
  if (!code && !reason) return '—';
  const key = code ? String(code) : '';
  const mapped = hangupCauseMap[key];
  if (mapped) {
    return `${code} ${reason ? `(${reason})` : ''} — ${mapped}`;
  }
  if (code && reason) {
    return `${code} - ${reason}`;
  }
  return String(code || reason || '—');
}

export const callDetailLabel = (key: string) => callDetailLabels[key] ?? '其他信息';

export function callDetailText(value: unknown, key?: string): string {
  if (value === null || value === undefined || value === '') return '—';
  if (key?.endsWith('duration_ms')) return `${durationSecondsText(value)} 秒`;
  if (key === 'billing_interval_secs') return `${valueText(value)} 秒`;
  if (key === 'price_per_interval' || (key && moneyFields.has(key)))
    return `${moneyText(value)} 元`;
  if (key?.endsWith('_at_ms')) {
    const milliseconds = Number(value);
    return Number.isFinite(milliseconds)
      ? new Date(milliseconds).toLocaleString('zh-CN', { hour12: false })
      : String(value);
  }
  if (key?.endsWith('_loss_rate')) return `${moneyText(Number(value) * 100)}%`;
  if (key?.endsWith('_jitter_ms') || key?.endsWith('_rtt_ms')) return `${moneyText(value)} 毫秒`;
  if (key === 'progress_percentage') return `${moneyText(value)}%`;
  if (typeof value === 'boolean') return value ? '是' : '否';
  if (Array.isArray(value)) return `${value.length} 项`;
  if (typeof value === 'object') return '查看关联状态';
  return callValueLabels[String(value)] ?? String(value);
}

export const entityId = (entity: Entity, key: string) => String(entity[key] ?? entity.id ?? '');

export { valueText, moneyText, durationSecondsText, datetimeText };
