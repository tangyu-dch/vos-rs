import { api, type PageResult } from '@/services/client';
import { saveSession, type AuthSession } from '@/services/auth';

export type Entity = Record<string, unknown> & { id?: string | number };

export async function login(username: string, password: string): Promise<AuthSession> {
  try {
    const result = await api.post<AuthSession & { access_token?: string }>('/auth/sessions', {
      username,
      password,
    });
    const token = result.access_token || result.token;
    if (
      !token ||
      !result.role ||
      !Array.isArray(result.permissions) ||
      !Array.isArray(result.menus)
    )
      throw new Error('登录响应缺少有效会话');
    const session: AuthSession = { ...result, token };
    saveSession(session);
    return session;
  } catch (err: any) {
    const reason = err?.response?.data?.message || err?.message || '用户名或密码错误';
    throw new Error(reason, { cause: err });
  }
}

export async function listResource<T extends Entity>(
  path: string,
  params: object = {},
  signal?: AbortSignal,
): Promise<PageResult<T>> {
  const result = await api.get<PageResult<T> | T[]>(path, params, signal);
  if (!Array.isArray(result)) return result;
  const page = Number((params as { page?: number }).page ?? 1);
  const pageSize = Number((params as { page_size?: number }).page_size ?? (result.length || 1));
  return {
    items: result,
    pagination: { page, page_size: pageSize, total: result.length, total_pages: 1 },
  };
}

export function getResource<T extends Entity>(path: string, id: string) {
  return api.get<T>(`${path}/${encodeURIComponent(id)}`);
}
export function createResource<T extends Entity>(path: string, body: Entity) {
  return api.post<T>(path, body);
}
export function updateResource<T extends Entity>(path: string, id: string, body: Entity) {
  return api.put<T>(`${path}/${encodeURIComponent(id)}`, body);
}
export function deleteResource(path: string, id: string) {
  return api.delete<void>(`${path}/${encodeURIComponent(id)}`);
}
