import { describe, expect, it } from 'vitest';
import { normalizeNotification } from '@/services/notifications';

describe('通知数据规范化', () => {
  it('兼容后端别名字段并识别未读状态', () => {
    const item = normalizeNotification({
      id: 42,
      notification_type: 'low_balance',
      level: 'high',
      title: '账户余额不足',
      content: '商户账户即将欠费',
      timestamp: '2026-07-29T10:00:00Z',
      read: false,
      link: '/billing/accounts',
    });

    expect(item).toMatchObject({
      id: '42',
      category: 'balance',
      severity: 'critical',
      message: '商户账户即将欠费',
      isRead: false,
      actionUrl: '/billing/accounts',
    });
  });

  it('未知类别降级为系统通知', () => {
    const item = normalizeNotification({
      id: 'one',
      category: 'unknown',
      read_at: '2026-07-29T10:00:00Z',
    });
    expect(item.category).toBe('system');
    expect(item.severity).toBe('info');
    expect(item.isRead).toBe(true);
    expect(item.title).toBe('系统通知');
  });

  it('已恢复告警显示为恢复状态', () => {
    const item = normalizeNotification({
      id: 'resolved-one',
      category: 'trunk',
      severity: 'critical',
      resolved: true,
    });
    expect(item.severity).toBe('success');
  });
});
