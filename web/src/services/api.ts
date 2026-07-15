import axios from 'axios';
import { clearSession, getAccessToken, saveSession, type AuthSession, isUserRole } from './auth';
import type {
  CdrEvent,
  DtmfEvent,
  SipUser,
  SipGateway,
  SipRoute,
  SipRegistration,
  DashboardStats,
  HourlyTrend,
  PaginatedResponse,
  ReportSummary,
  BillingRate,
  BillingAccount,
  LedgerEntry,
  ReconcileResult,
  ActiveCall,
  NumberInventory,
  AntiFraudRule,
  AntiFraudConfigItem,
  AuditLog,
} from '@/types';

export interface MediaClusterNode {
  id: string;
  type: 'local' | 'remote';
  control_url?: string;
  advertised_addr: string;
  port_min: number;
  port_max: number;
  weight: number;
  control_token?: string;
  control_token_configured: boolean;
}

export interface MediaClusterConfig {
  allocation_strategy: 'weighted_round_robin' | 'least_sessions' | 'call_id_hash';
  health_check_interval_secs: number;
  unhealthy_threshold: number;
  nodes: MediaClusterNode[];
}

export interface SipClusterNodeStatus {
  node_id: string;
  advertised_addr: string;
  management_url: string;
  router_mode: 'direct' | 'external' | 'native';
  status: 'active' | 'draining';
  active_calls: number;
  version: string;
  started_at: number;
  updated_at: number;
  ttl_secs: number;
}

export interface SipClusterStatus {
  node_key_prefix: string;
  online_nodes: number;
  active_nodes: number;
  draining_nodes: number;
  nodes: SipClusterNodeStatus[];
}

export interface SipClusterNodeActionResult {
  status: 'active' | 'draining';
  active_calls: number;
}

const api = axios.create({
  baseURL: '/api',
  timeout: 30000,
  headers: {
    'Content-Type': 'application/json',
  },
});

interface ApiErrorBody {
  error?: string;
  message?: string;
  details?: string;
}

/** 统一的 API 错误，保留 HTTP 状态码供页面执行鉴权和重试逻辑。 */
export class ApiError extends Error {
  readonly status?: number;
  readonly code?: string;
  readonly requestId?: string;

  constructor(message: string, options: { status?: number; code?: string; requestId?: string } = {}) {
    super(message);
    this.name = 'ApiError';
    this.status = options.status;
    this.code = options.code;
    this.requestId = options.requestId;
  }
}

/** 将后端错误统一转换成页面可直接展示的中文错误，并保留请求编号。 */
export function formatApiError(error: unknown): ApiError | Error {
  if (error instanceof ApiError) {
    return error;
  }
  if (error instanceof Error && !(error as Error & { response?: unknown }).response) {
    return error;
  }
  const response = (error as { response?: { status?: number; data?: ApiErrorBody; headers?: Record<string, string> } } | null)?.response;
  const body = response?.data;
  const message = body?.error || body?.message || body?.details || '请求失败，请稍后重试';
  const requestId = response?.headers?.['x-request-id'] || response?.headers?.['X-Request-ID'];
  return new ApiError(requestId ? `${message}（请求 ID: ${requestId}）` : message, {
    status: response?.status,
    requestId,
  });
}

api.interceptors.request.use((config) => {
  const token = getAccessToken();
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

api.interceptors.response.use(
  (response) => response,
  (error) => {
    if (error.response?.status === 401 && window.location.pathname !== '/login') {
      clearSession();
      window.location.assign('/login');
    }
    return Promise.reject(formatApiError(error));
  },
);

// ===== 仪表板 API =====

export const apiService = {
  async login(username: string, password: string): Promise<AuthSession> {
    const response = await api.post<{ token: string; username: string; role: string }>(
      '/auth/login',
      { username, password },
    );
    if (!isUserRole(response.data.role)) {
      throw new Error('服务器返回了未知角色');
    }
    const session: AuthSession = {
      token: response.data.token,
      username: response.data.username,
      role: response.data.role,
    };
    saveSession(session);
    return session;
  },

  // 获取仪表板统计
  async getDashboardStats(): Promise<DashboardStats> {
    const response = await api.get<DashboardStats>('/dashboard/stats');
    return response.data;
  },

  // 获取今日按小时呼叫趋势
  async getHourlyTrend(): Promise<HourlyTrend[]> {
    const response = await api.get<HourlyTrend[]>('/dashboard/trend');
    return response.data;
  },

  // 获取最近的 CDR（从列表中取前10条）
  async getRecentCdrs(limit: number = 10): Promise<CdrEvent[]> {
    const response = await api.get<PaginatedResponse<CdrEvent>>('/cdrs', {
      params: { page: 1, page_size: limit }
    });
    return response.data.items;
  },

  // ===== CDR API =====

  async getCdrs(params: {
    page: number;
    page_size: number;
    call_id?: string;
    status?: string;
    caller?: string;
    callee?: string;
    start_time?: string;
    end_time?: string;
    signal?: AbortSignal;
  }): Promise<PaginatedResponse<CdrEvent>> {
    const { signal, ...query } = params;
    const response = await api.get<PaginatedResponse<CdrEvent>>('/cdrs', { params: query, signal });
    return response.data;
  },

  async getCdr(callId: string): Promise<CdrEvent | null> {
    const response = await api.get<CdrEvent | null>(`/cdrs/${callId}`);
    return response.data;
  },

  async getDtmfEvents(callId: string): Promise<DtmfEvent[]> {
    const response = await api.get<DtmfEvent[]>(`/cdrs/${callId}/dtmf`);
    return response.data;
  },

  // ===== SIP 用户 API =====

  async getUsers(page = 1, pageSize = 20): Promise<PaginatedResponse<SipUser>> {
    const response = await api.get<PaginatedResponse<SipUser>>('/users', {
      params: { page, page_size: pageSize },
    });
    return response.data;
  },

  async createUser(user: { username: string; password: string }): Promise<void> {
    await api.post('/users', user);
  },

  async updateUser(username: string, password: string): Promise<void> {
    await api.put(`/users/${username}`, { password });
  },

  async deleteUser(username: string): Promise<void> {
    await api.delete(`/users/${username}`);
  },

  // ===== 网关 API =====

  async getGateways(page = 1, pageSize = 20, gatewayType?: string): Promise<PaginatedResponse<SipGateway>> {
    const response = await api.get<PaginatedResponse<SipGateway>>('/gateways', {
      params: { page, page_size: pageSize, gateway_type: gatewayType },
    });
    return response.data;
  },

  async createGateway(gateway: SipGateway): Promise<void> {
    await api.post('/gateways', gateway);
  },

  async updateGateway(id: string, gateway: Omit<SipGateway, 'id' | 'created_at'>): Promise<void> {
    await api.put(`/gateways/${id}`, gateway);
  },

  async deleteGateway(id: string): Promise<void> {
    await api.delete(`/gateways/${id}`);
  },

  // ===== 路由 API =====

  async getRoutes(page = 1, pageSize = 20): Promise<PaginatedResponse<SipRoute>> {
    const response = await api.get<PaginatedResponse<SipRoute>>('/routes', {
      params: { page, page_size: pageSize },
    });
    return response.data;
  },

  async createRoute(route: SipRoute): Promise<void> {
    await api.post('/routes', route);
  },

  async updateRoute(id: string, route: Omit<SipRoute, 'id' | 'created_at'>): Promise<void> {
    await api.put(`/routes/${id}`, route);
  },

  async deleteRoute(id: string): Promise<void> {
    await api.delete(`/routes/${id}`);
  },

  // ===== 注册信息 API =====

  async getRegistrations(page = 1, pageSize = 20, keyword?: string): Promise<PaginatedResponse<SipRegistration>> {
    const response = await api.get<PaginatedResponse<SipRegistration>>('/registrations', {
      params: { page, page_size: pageSize, keyword },
    });
    return response.data;
  },

  // 录音只作为 CDR 详情的附属资源提供，不再维护独立录音列表。
  async getRecordingAudio(callId: string): Promise<Blob> {
    const response = await api.get<Blob>(`/recordings/${encodeURIComponent(callId)}/audio`, {
      responseType: 'blob',
    });
    return response.data;
  },

  // ===== 报表 =====
  async getReportSummary(start?: string, end?: string): Promise<ReportSummary> {
    const r = await api.get<ReportSummary>('/reports/summary', {
      params: { start_time: start, end_time: end },
    });
    return r.data;
  },
  async exportReport(start?: string, end?: string): Promise<Blob> {
    const response = await api.get<Blob>('/reports/export', {
      params: { start_time: start, end_time: end },
      responseType: 'blob',
    });
    return response.data;
  },

  // ===== 计费：费率 =====
  async getRates(page = 1, pageSize = 20): Promise<PaginatedResponse<BillingRate>> {
    const r = await api.get<PaginatedResponse<BillingRate>>('/rates', {
      params: { page, page_size: pageSize },
    });
    return r.data;
  },
  async createRate(rate: BillingRate): Promise<void> {
    await api.post('/rates', {
      id: rate.id,
      prefix: rate.prefix,
      rate_per_minute: rate.rate_per_minute,
      description: rate.description,
    });
  },
  async updateRate(id: string, rate: Omit<BillingRate, 'id' | 'created_at'>): Promise<void> {
    await api.put(`/rates/${id}`, {
      prefix: rate.prefix,
      rate_per_minute: rate.rate_per_minute,
      description: rate.description,
    });
  },
  async deleteRate(id: string): Promise<void> {
    await api.delete(`/rates/${id}`);
  },

  // ===== 计费：账户 =====
  async getAccounts(page = 1, pageSize = 20): Promise<PaginatedResponse<BillingAccount>> {
    const r = await api.get<PaginatedResponse<BillingAccount>>('/accounts', {
      params: { page, page_size: pageSize },
    });
    return r.data;
  },
  async creditAccount(
    username: string,
    amount: number
  ): Promise<{ username: string; balance: number }> {
    const r = await api.post(`/accounts/${username}/credit`, { amount });
    return r.data;
  },
  async getLedger(username?: string, page = 1, pageSize = 20): Promise<PaginatedResponse<LedgerEntry>> {
    const r = await api.get<PaginatedResponse<LedgerEntry>>('/ledger', {
      params: { username, page, page_size: pageSize },
    });
    return r.data;
  },
  async reconcileBilling(start?: string, end?: string): Promise<ReconcileResult> {
    const r = await api.post('/billing/reconcile', null, {
      params: { start_time: start, end_time: end },
    });
    return r.data;
  },

  // ===== 活跃呼叫（转发到 sip-edge 管理 API） =====
  async getActiveCalls(): Promise<ActiveCall[]> {
    const r = await api.get<ActiveCall[]>('/calls/active');
    return r.data;
  },
  async terminateCall(callId: string): Promise<void> {
    await api.post(`/calls/${encodeURIComponent(callId)}/terminate`);
  },
  async routePreview(destination: string): Promise<{
    destination: string;
    candidates: { route_id: string; gateway_id: string; host: string; port: number | null }[];
    error?: string;
  }> {
    const r = await api.get('/route-preview', { params: { destination } });
    return r.data;
  },

  // ===== 号码库存 =====
  async getNumbers(page = 1, pageSize = 20): Promise<PaginatedResponse<NumberInventory>> {
    const r = await api.get<PaginatedResponse<NumberInventory>>('/numbers', {
      params: { page, page_size: pageSize },
    });
    return r.data;
  },
  async createNumber(n: {
    number: string;
    username?: string;
    gateway_id?: string;
    direction?: string;
    max_concurrent?: number;
    status: string;
  }): Promise<void> {
    await api.post('/numbers', n);
  },
  async updateNumber(
    number: string,
    body: { username?: string; gateway_id?: string; direction?: string; max_concurrent?: number; status?: string }
  ): Promise<void> {
    await api.put(`/numbers/${encodeURIComponent(number)}`, body);
  },
  async deleteNumber(number: string): Promise<void> {
    await api.delete(`/numbers/${encodeURIComponent(number)}`);
  },

  // ===== 防盗打 =====
  async getAntiFraudRules(): Promise<AntiFraudRule[]> {
    const r = await api.get<AntiFraudRule[]>('/anti-fraud/rules');
    return r.data;
  },
  async getAntiFraudConfig(): Promise<AntiFraudConfigItem[]> {
    const r = await api.get<AntiFraudConfigItem[]>('/anti-fraud/config');
    return r.data;
  },
  async updateAntiFraudConfig(key: string, configValue: string): Promise<void> {
    await api.put(`/anti-fraud/config/${encodeURIComponent(key)}`, {
      config_value: configValue,
    });
  },
  async getAuditLogs(page = 1, pageSize = 50): Promise<PaginatedResponse<AuditLog>> {
    const response = await api.get<PaginatedResponse<AuditLog>>('/audit-logs', {
      params: { page, page_size: pageSize },
    });
    return response.data;
  },
  async createAntiFraudRule(rule: { id: string; rule_type: string; target_value: string; limit_number: number | null; enabled: boolean }): Promise<void> {
    await api.post('/anti-fraud/rules', rule);
  },
  async updateAntiFraudRule(id: string, data: { rule_type: string; target_value: string; limit_number: number | null; enabled: boolean }): Promise<void> {
    await api.put(`/anti-fraud/rules/${id}`, data);
  },
  async deleteAntiFraudRule(id: string): Promise<void> {
    await api.delete(`/anti-fraud/rules/${id}`);
  },

  // ===== 系统配置 API =====
  async getSystemConfigs(): Promise<Record<string, string>> {
    const response = await api.get<{ configs: Record<string, string> }>('/system/configs');
    return response.data.configs;
  },
  async updateSystemConfigs(configs: Record<string, string>): Promise<void> {
    await api.post('/system/configs', configs);
  },
  async getMediaCluster(): Promise<MediaClusterConfig> {
    const response = await api.get<MediaClusterConfig>('/system/media-cluster');
    return response.data;
  },
  async updateMediaCluster(config: MediaClusterConfig): Promise<MediaClusterConfig> {
    const response = await api.put<MediaClusterConfig>('/system/media-cluster', config);
    return response.data;
  },
  async getSipClusterStatus(): Promise<SipClusterStatus> {
    const response = await api.get<SipClusterStatus>('/system/sip-cluster/status');
    return response.data;
  },
  async controlSipClusterNode(nodeId: string, action: 'drain' | 'resume'): Promise<SipClusterNodeActionResult> {
    const response = await api.post<SipClusterNodeActionResult>(
      `/system/sip-cluster/nodes/${encodeURIComponent(nodeId)}/${action}`,
    );
    return response.data;
  },
};

export default api;
