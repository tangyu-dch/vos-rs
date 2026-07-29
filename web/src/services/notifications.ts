import { api, type Pagination } from '@/services/client';

export type NotificationCategory =
  'server' | 'trunk' | 'registration' | 'balance' | 'call_quality' | 'risk' | 'security' | 'system';

export type NotificationSeverity = 'critical' | 'warning' | 'info' | 'success';

export interface NotificationItem {
  id: string;
  category: NotificationCategory;
  severity: NotificationSeverity;
  title: string;
  message: string;
  createdAt: string;
  isRead: boolean;
  actionUrl?: string;
}

interface NotificationRecord {
  id: string | number;
  category?: string;
  notification_type?: string;
  type?: string;
  severity?: string;
  level?: string;
  title?: string;
  message?: string;
  content?: string;
  description?: string;
  created_at?: string;
  timestamp?: string;
  is_read?: boolean;
  read?: boolean;
  read_at?: string | null;
  resolved?: boolean;
  action_url?: string;
  link?: string;
}

interface NotificationListPayload {
  items?: NotificationRecord[];
  notifications?: NotificationRecord[];
  pagination?: Pagination;
  total?: number;
  unread?: number;
  page?: number;
  page_size?: number;
}

interface UnreadCountPayload {
  unread_count?: number;
  count?: number;
  unread?: number;
}

export interface NotificationListResult {
  items: NotificationItem[];
  pagination?: Pagination;
  total?: number;
  unread?: number;
}

const CATEGORY_ALIASES: Record<string, NotificationCategory> = {
  server: 'server',
  server_alert: 'server',
  infrastructure: 'server',
  trunk: 'trunk',
  trunk_alert: 'trunk',
  registration: 'registration',
  registration_alert: 'registration',
  balance: 'balance',
  low_balance: 'balance',
  billing: 'balance',
  call_quality: 'call_quality',
  media_quality: 'call_quality',
  risk: 'risk',
  risk_control: 'risk',
  fraud: 'risk',
  anti_fraud: 'risk',
  security: 'security',
  system: 'system',
};

function normalizeCategory(value: string | undefined): NotificationCategory {
  return CATEGORY_ALIASES[String(value || '').toLowerCase()] ?? 'system';
}

function normalizeSeverity(value: string | undefined): NotificationSeverity {
  switch (String(value || '').toLowerCase()) {
    case 'critical':
    case 'error':
    case 'danger':
    case 'high':
      return 'critical';
    case 'warning':
    case 'warn':
    case 'medium':
      return 'warning';
    case 'success':
    case 'resolved':
      return 'success';
    default:
      return 'info';
  }
}

function defaultActionUrl(category: NotificationCategory): string {
  const routes: Record<NotificationCategory, string> = {
    server: '/infrastructure',
    trunk: '/trunks/egress',
    registration: '/extensions',
    balance: '/billing/accounts',
    call_quality: '/calls',
    risk: '/security',
    security: '/security',
    system: '/overview',
  };
  return routes[category];
}

export function normalizeNotification(record: NotificationRecord): NotificationItem {
  const category = normalizeCategory(record.category ?? record.notification_type ?? record.type);
  return {
    id: String(record.id),
    category,
    severity: record.resolved ? 'success' : normalizeSeverity(record.severity ?? record.level),
    title: record.title?.trim() || '系统通知',
    message:
      record.message?.trim() ||
      record.content?.trim() ||
      record.description?.trim() ||
      '暂无详细内容',
    createdAt: record.created_at ?? record.timestamp ?? new Date().toISOString(),
    isRead: Boolean(record.is_read ?? record.read ?? record.read_at),
    actionUrl: record.action_url ?? record.link ?? defaultActionUrl(category),
  };
}

/** 获取站内通知，兼容直接数组以及 items、notifications 两种列表结构。 */
export async function getNotifications(
  unreadOnly: boolean,
  page = 1,
  pageSize = 20,
): Promise<NotificationListResult> {
  const payload = await api.get<NotificationListPayload | NotificationRecord[]>('/notifications', {
    unread_only: unreadOnly,
    page,
    page_size: pageSize,
  });
  if (Array.isArray(payload)) return { items: payload.map(normalizeNotification) };
  const records = payload.items ?? payload.notifications ?? [];
  return {
    items: records.map(normalizeNotification),
    pagination: payload.pagination,
    total: payload.total ?? payload.pagination?.total,
    unread: payload.unread,
  };
}

/** 获取当前用户的未读通知数量。 */
export async function getUnreadNotificationCount(): Promise<number> {
  const payload = await api.get<UnreadCountPayload | number>('/notifications/unread-count');
  if (typeof payload === 'number') return Math.max(0, payload);
  return Math.max(0, payload.unread_count ?? payload.unread ?? payload.count ?? 0);
}

/** 将一条通知标记为已读。 */
export async function markNotificationRead(id: string): Promise<void> {
  await api.post(`/notifications/${encodeURIComponent(id)}/read`);
}

/** 将当前用户的全部通知标记为已读。 */
export async function markAllNotificationsRead(): Promise<void> {
  await api.post('/notifications/read-all');
}
