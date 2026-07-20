import { describe, expect, it } from 'vitest';
import { callDetailLabel, callDetailText } from '@/pages/shared/format';

describe('call detail presentation', () => {
  it('uses Chinese labels instead of exposing CDR snake case fields', () => {
    expect(callDetailLabel('billable_duration_ms')).toBe('计费时长');
    expect(callDetailLabel('gateway_rtcp_jitter_ms')).toBe('落地抖动');
    expect(callDetailLabel('future_backend_field')).toBe('其他信息');
  });

  it('shows call durations in seconds and money with at most three decimals', () => {
    expect(callDetailText(45_000, 'billable_duration_ms')).toBe('45 秒');
    expect(callDetailText(6, 'billing_interval_secs')).toBe('6 秒');
    expect(callDetailText(0.050_000_1, 'price_per_interval')).toBe('0.05 元');
    expect(callDetailText(12.345_67, 'amount')).toBe('12.346 元');
  });

  it('localizes call list values instead of exposing protocol values', () => {
    expect(callDetailText('outbound', 'direction')).toBe('呼出');
    expect(callDetailText('answered', 'status')).toBe('已接通');
    expect(callDetailText(1_784_216_708_295, 'started_at_ms')).not.toContain('1784216708295');
  });

  it('translates routing audit decisions for operators', () => {
    expect(callDetailText('trunk', 'source_type')).toBe('接入中继');
    expect(callDetailText('strict_passthrough', 'caller_mode')).toBe('严格透传');
    expect(callDetailText('virtual_pool', 'caller_mode')).toBe('号码池主叫');
    expect(callDetailText('round_robin', 'caller_selection')).toBe('顺序轮询');
    expect(callDetailText(true, 'fallback_used')).toBe('是');
  });
});
