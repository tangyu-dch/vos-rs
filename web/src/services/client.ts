import axios, { type AxiosRequestConfig } from 'axios';
import { clearSession, getAccessToken } from './auth';

interface Envelope<T> { code: number; message: string; data: T; request_id: string; }
interface ErrorEnvelope { code?: number | string; message?: string; details?: string; request_id?: string; }
export interface Pagination { page: number; page_size: number; total: number; total_pages: number; }
export interface PageResult<T> { items: T[]; pagination: Pagination; }

export class ApiError extends Error {
  constructor(message: string, readonly status?: number, readonly code?: string, readonly requestId?: string) {
    super(message); this.name = 'ApiError';
  }
}

const http = axios.create({ baseURL: '/api/v1', timeout: 30000, headers: { 'Content-Type': 'application/json' } });
http.interceptors.request.use((config) => {
  const token = getAccessToken();
  if (token) config.headers.Authorization = `Bearer ${token}`;
  return config;
});
http.interceptors.response.use(undefined, (error) => {
  const status = error.response?.status as number | undefined;
  const body = error.response?.data as ErrorEnvelope | undefined;
  if (status === 401 && window.location.pathname !== '/login') { clearSession(); window.location.assign('/login'); }
  const message = body?.details || body?.message || (error.code === 'ECONNABORTED' ? '请求超时，请检查服务状态' : '请求失败，请稍后重试');
  return Promise.reject(new ApiError(message, status, String(body?.code ?? ''), body?.request_id));
});

export function unwrap<T>(payload: Envelope<T> | T): T {
  if (payload && typeof payload === 'object' && 'data' in payload && 'code' in payload) {
    const envelope = payload as Envelope<T>;
    if (envelope.code !== 0) throw new ApiError(envelope.message, undefined, String(envelope.code), envelope.request_id);
    return envelope.data;
  }
  return payload as T;
}

export async function request<T>(config: AxiosRequestConfig): Promise<T> {
  const response = await http.request<Envelope<T> | T>(config);
  return unwrap(response.data);
}

export const api = {
  get: <T>(url: string, params?: object, signal?: AbortSignal) => request<T>({ method: 'GET', url, params, signal }),
  post: <T>(url: string, data?: unknown) => request<T>({ method: 'POST', url, data }),
  patch: <T>(url: string, data?: unknown) => request<T>({ method: 'PATCH', url, data }),
  put: <T>(url: string, data?: unknown) => request<T>({ method: 'PUT', url, data }),
  delete: <T>(url: string) => request<T>({ method: 'DELETE', url }),
  blob: async (url: string) => {
    const response = await http.get<Blob>(url, { responseType: 'blob' });
    return response.data;
  },
};
