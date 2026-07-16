import { api, type PageResult } from './client';
import { isUserRole, saveSession, type AuthSession } from './auth';

export type Entity = Record<string, unknown> & { id?: string | number };

export async function login(username: string, password: string): Promise<AuthSession> {
  const result = await api.post<{ access_token?: string; token?: string; username: string; role: string }>('/auth/sessions', { username, password });
  const token = result.access_token || result.token;
  if (!token || !isUserRole(result.role)) throw new Error('登录响应缺少有效会话');
  const session: AuthSession = { token, username: result.username, role: result.role };
  saveSession(session);
  return session;
}

export async function listResource<T extends Entity>(path: string, params: object = {}, signal?: AbortSignal): Promise<PageResult<T>> {
  const result = await api.get<PageResult<T> | T[]>(path, params, signal);
  if (!Array.isArray(result)) return result;
  const page = Number((params as { page?: number }).page ?? 1);
  const pageSize = Number((params as { page_size?: number }).page_size ?? (result.length || 1));
  return { items: result, pagination: { page, page_size: pageSize, total: result.length, total_pages: 1 } };
}
export function getResource<T extends Entity>(path: string, id: string) { return api.get<T>(`${path}/${encodeURIComponent(id)}`); }
export function createResource<T extends Entity>(path: string, body: Entity) { return api.post<T>(path, body); }
export function updateResource<T extends Entity>(path: string, id: string, body: Entity) { return api.put<T>(`${path}/${encodeURIComponent(id)}`, body); }
export function deleteResource(path: string, id: string) { return api.delete<void>(`${path}/${encodeURIComponent(id)}`); }
