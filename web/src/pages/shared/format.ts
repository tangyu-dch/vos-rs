// 通话详情/资源字段的展示格式化工具
// 从 console.tsx 拆分

import type { Entity } from '@/services/resources';

const valueText = (value: unknown) =>
  value === null || value === undefined || value === '' ? '—' : String(value);

const moneyFields = new Set([
  'balance', 'credit_limit', 'price_per_interval', 'amount', 'balance_after', 'cost',
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
  return (milliseconds / 1000).toLocaleString('zh-CN', {
    minimumFractionDigits: 0,
    maximumFractionDigits: 3,
  });
};

const callDetailLabels: Record<string, string> = {
  call_id: '通话 ID', caller: '主叫号码', callee: '被叫号码', started_at_ms: '开始时间',
  answered_at_ms: '接通时间', ended_at_ms: '结束时间', duration_ms: '通话时长',
  billable_duration_ms: '计费时长', status: '通话状态', failure_status_code: '失败状态码',
  failure_reason: '失败原因', caller_rtcp_loss_rate: '主叫丢包率', caller_rtcp_jitter_ms: '主叫抖动',
  caller_rtcp_rtt_ms: '主叫往返时延', gateway_rtcp_loss_rate: '落地丢包率',
  gateway_rtcp_jitter_ms: '落地抖动', gateway_rtcp_rtt_ms: '落地往返时延', mos: '通话质量 MOS',
  dtmf_digits: '按键记录', recording_path: '录音文件', direction: '呼叫方向', state: '实时状态',
  gateway: '当前中继', muted: '静音状态', playback: '放音状态', file_path: '音频文件',
  mode: '播放模式', loop_playback: '循环播放', progress_percentage: '播放进度',
  runtime_availability: '实时状态', digit: '按键', source: '事件来源', timestamp_ms: '发生时间',
  rtp_timestamp: 'RTP 时间戳', volume: '音量', inserted_at: '写入时间',
  id: '资源标识', name: '资源名称', username: '用户账号', created_at: '创建时间',
  updated_at: '更新时间', enabled: '启用状态', host: '主机地址', port: '服务端口',
  transport: '传输协议', role: '资源类型', description: '说明', max_capacity: '容量上限',
  current_concurrent: '当前并发', max_concurrent: '最大并发', number: '号码',
};

const callValueLabels: Record<string, string> = {
  answered: '已接通', canceled: '已取消', failed: '失败', inbound: '呼入', outbound: '呼出',
  trunk: '接入中继', extension: '分机号码', passthrough: '透传主叫', strict_passthrough: '严格透传',
  fixed: '固定主叫', fixed_number: '固定号码',
  virtual_pool: '号码池主叫', random: '均匀随机', weighted_random: '权重随机',
  round_robin: '顺序轮询', stable_hash: '稳定哈希', available: '实时可用',
  not_active: '通话已结束', unavailable: '控制面不可用', rtp: 'RTP 事件', 'sip-info': 'SIP INFO',
};

export const callDetailLabel = (key: string) => callDetailLabels[key] ?? '其他信息';

export function callDetailText(value: unknown, key?: string): string {
  if (value === null || value === undefined || value === '') return '—';
  if (key?.endsWith('duration_ms')) return `${durationSecondsText(value)} 秒`;
  if (key === 'billing_interval_secs') return `${valueText(value)} 秒`;
  if (key === 'price_per_interval' || (key && moneyFields.has(key))) return `${moneyText(value)} 元`;
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

export { valueText, moneyText, durationSecondsText };
