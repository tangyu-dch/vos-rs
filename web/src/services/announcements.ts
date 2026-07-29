import { api, type Pagination } from '@/services/client';

export type AnnouncementCategory = 'system' | 'maintenance' | 'business' | 'security';
export type AnnouncementStatus = 'draft' | 'published';
export type AnnouncementTarget = 'all' | 'specified';
export type AnnouncementDelivery = 'system' | 'popup';

export interface Announcement {
  id: string;
  title: string;
  category: AnnouncementCategory;
  audience: AnnouncementTarget;
  audience_users: string[];
  delivery_methods: AnnouncementDelivery[];
  scheduled_at: string | null;
  pinned: boolean;
  content: string;
  status: AnnouncementStatus;
  created_at: string;
  updated_at?: string;
  published_at?: string | null;
  publisher?: string | null;
}

export interface MyAnnouncement extends Announcement {
  is_read: boolean;
  read_at?: string | null;
}

export interface AnnouncementInput {
  title: string;
  category: AnnouncementCategory;
  audience: AnnouncementTarget;
  audience_users: string[];
  delivery_methods: AnnouncementDelivery[];
  scheduled_at: string | null;
  pinned: boolean;
  content: string;
}

export interface AnnouncementQuery {
  q?: string;
  category?: string;
  status?: string;
  unread_only?: boolean;
  page?: number;
  page_size?: number;
}

export interface AnnouncementList<T> {
  items: T[];
  pagination?: Pagination;
  total: number;
}

interface AnnouncementPayload<T> {
  items?: T[];
  announcements?: T[];
  pagination?: Pagination;
  total?: number;
}

function normalizeList<T>(payload: AnnouncementPayload<T> | T[]): AnnouncementList<T> {
  if (Array.isArray(payload)) return { items: payload, total: payload.length };
  const items = payload.items ?? payload.announcements ?? [];
  return {
    items,
    pagination: payload.pagination,
    total: payload.total ?? payload.pagination?.total ?? items.length,
  };
}

/** 查询公告管理列表。 */
export async function getAnnouncements(
  query: AnnouncementQuery = {},
): Promise<AnnouncementList<Announcement>> {
  return normalizeList(
    await api.get<AnnouncementPayload<Announcement> | Announcement[]>('/announcements', query),
  );
}

/** 创建公告草稿。 */
export async function createAnnouncement(input: AnnouncementInput): Promise<Announcement> {
  return api.post<Announcement>('/announcements', input);
}

/** 更新公告内容。 */
export async function updateAnnouncement(
  id: string,
  input: AnnouncementInput,
): Promise<Announcement> {
  return api.put<Announcement>(`/announcements/${encodeURIComponent(id)}`, input);
}

/** 删除公告。 */
export async function deleteAnnouncement(id: string): Promise<void> {
  await api.delete(`/announcements/${encodeURIComponent(id)}`);
}

/** 发布公告；定时公告由后端根据发布时间进入待发布状态。 */
export async function publishAnnouncement(id: string): Promise<Announcement> {
  return api.post<Announcement>(`/announcements/${encodeURIComponent(id)}/publish`);
}

/** 查询当前用户收到的公告。 */
export async function getMyAnnouncements(
  query: AnnouncementQuery = {},
): Promise<AnnouncementList<MyAnnouncement>> {
  return normalizeList(
    await api.get<AnnouncementPayload<MyAnnouncement> | MyAnnouncement[]>(
      '/my-announcements',
      query,
    ),
  );
}

/** 将一条个人公告标记为已读。 */
export async function markMyAnnouncementRead(id: string): Promise<void> {
  await api.post(`/my-announcements/${encodeURIComponent(id)}/read`);
}
