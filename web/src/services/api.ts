import axios from 'axios';
import { clearSession, getAccessToken, saveSession, type AuthSession, isUserRole } from './auth';
import type {
  CdrEvent,
  SipUser,
  SipGateway,
  SipRoute,
  SipRegistration,
  DashboardStats,
  HourlyTrend,
  PaginatedResponse,
  RecordingInfo,
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

const api = axios.create({
  baseURL: '/api',
  timeout: 30000,
  headers: {
    'Content-Type': 'application/json',
  },
});

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
    return Promise.reject(error);
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
    status?: string;
    caller?: string;
    callee?: string;
    start_time?: string;
    end_time?: string;
  }): Promise<PaginatedResponse<CdrEvent>> {
    const response = await api.get<PaginatedResponse<CdrEvent>>('/cdrs', { params });
    return response.data;
  },

  async getCdr(callId: string): Promise<CdrEvent | null> {
    const response = await api.get<CdrEvent | null>(`/cdrs/${callId}`);
    return response.data;
  },

  async getDtmfEvents(callId: string): Promise<any[]> {
    const response = await api.get<any[]>(`/cdrs/${callId}/dtmf`);
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

  async getRegistrations(): Promise<SipRegistration[]> {
    const response = await api.get<SipRegistration[]>('/registrations');
    return response.data;
  },

  // ===== 录音 =====
  async getRecordings(): Promise<RecordingInfo[]> {
    const r = await api.get<RecordingInfo[]>('/recordings');
    return r.data;
  },
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
  async getRates(): Promise<BillingRate[]> {
    const r = await api.get<BillingRate[]>('/rates');
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
};

export default api;
