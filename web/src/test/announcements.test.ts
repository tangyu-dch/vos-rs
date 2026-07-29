import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '@/services/client';
import {
  createAnnouncement,
  getAnnouncements,
  getMyAnnouncements,
  markMyAnnouncementRead,
  publishAnnouncement,
  type AnnouncementInput,
} from '@/services/announcements';

const input: AnnouncementInput = {
  title: '系统维护通知',
  category: 'maintenance',
  audience: 'all',
  audience_users: [],
  delivery_methods: ['system', 'popup'],
  scheduled_at: null,
  pinned: true,
  content: '今晚进行维护。',
};

describe('公告服务', () => {
  afterEach(() => vi.restoreAllMocks());

  it('兼容分页公告列表结构', async () => {
    vi.spyOn(api, 'get').mockResolvedValueOnce({ items: [{ id: 'one' }], total: 9 });
    const result = await getAnnouncements({ q: '维护', page: 1, page_size: 20 });
    expect(result.total).toBe(9);
    expect(result.items[0]).toMatchObject({ id: 'one' });
    expect(api.get).toHaveBeenCalledWith('/announcements', { q: '维护', page: 1, page_size: 20 });
  });

  it('创建和发布使用约定端点', async () => {
    vi.spyOn(api, 'post')
      .mockResolvedValueOnce({ id: 'notice-1' })
      .mockResolvedValueOnce({ id: 'notice-1', status: 'published' });
    await createAnnouncement(input);
    await publishAnnouncement('notice-1');
    expect(api.post).toHaveBeenNthCalledWith(1, '/announcements', input);
    expect(api.post).toHaveBeenNthCalledWith(2, '/announcements/notice-1/publish');
  });

  it('个人公告列表和已读端点相互独立', async () => {
    vi.spyOn(api, 'get').mockResolvedValueOnce([{ id: 'my-one', is_read: false }]);
    vi.spyOn(api, 'post').mockResolvedValueOnce(undefined);
    const result = await getMyAnnouncements({ unread_only: true });
    await markMyAnnouncementRead('my-one');
    expect(result.total).toBe(1);
    expect(api.get).toHaveBeenCalledWith('/my-announcements', { unread_only: true });
    expect(api.post).toHaveBeenCalledWith('/my-announcements/my-one/read');
  });
});
