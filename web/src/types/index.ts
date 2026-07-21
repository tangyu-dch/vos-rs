export interface CdrAuditSnapshot {
  source_type?: string;
  source_id?: string;
  billing_account?: string;
  original_caller?: string;
  presented_caller?: string;
  caller_mode?: string;
  caller_pool_id?: string;
  caller_selection?: string;
  ingress_trunk_id?: string;
  egress_trunk_id?: string;
  selected_route_id?: string;
  fallback_used: boolean;
  billing_interval_secs?: number;
  price_per_interval?: number;
}

export interface CdrEvent {
  call_id: string;
  caller?: string;
  callee?: string;
  started_at_ms: number;
  answered_at_ms?: number;
  ended_at_ms: number;
  duration_ms: number;
  billable_duration_ms: number;
  status: 'answered' | 'canceled' | 'failed';
  failure_status_code?: number;
  failure_reason?: string;
  caller_rtcp_loss_rate?: number;
  caller_rtcp_jitter_ms?: number;
  caller_rtcp_rtt_ms?: number;
  gateway_rtcp_loss_rate?: number;
  gateway_rtcp_jitter_ms?: number;
  gateway_rtcp_rtt_ms?: number;
  mos?: number;
  dtmf_digits?: string;
  recording_path?: string | null;
  direction?: 'inbound' | 'outbound';
  audit?: CdrAuditSnapshot;
}

export interface SipUser {
  username: string;
  password?: string;
  created_at?: string;
}

export interface SipGateway {
  id: string;
  host: string;
  port?: number;
  transport: string;
  max_capacity?: number;
  gateway_type?: 'gateway' | 'peer' | 'extension';
  prefix_rules?: string;
  supports_registration?: boolean;
  reg_auth_type?: string;
  reg_username?: string;
  parent_gateway_id?: string;
  caller_id_mode?: string;
  virtual_caller?: string;
  current_concurrent?: number;
  circuit_state?: 'closed' | 'open' | 'half_open' | string;
  account_id?: number;
  max_concurrent?: number;
  enabled?: boolean;
  created_at?: string;
}

export interface SipRoute {
  id: string;
  prefix: string;
  priority: number;
  gateway_id: string;
  cost: number;
  weight: number;
  time_start?: string;
  time_end?: string;
  created_at?: string;
}

export interface SipRegistration {
  aor: string;
  contact_uri: string;
  received_from: string;
  expires_at: string;
  path: string[];
  updated_at?: string;
  contact?: string;
  user_agent?: string;
  node?: string;
}

export interface DtmfEvent {
  call_id: string;
  digit: string;
  source: 'rtp' | 'sip-info';
  timestamp_ms: number;
  rtp_timestamp?: number;
  duration_ms?: number;
  volume?: number;
  inserted_at?: string;
}

export interface DashboardStats {
  active_calls: number;
  today_total_calls: number;
  today_answered_calls: number;
  today_canceled_calls: number;
  today_failed_calls: number;
  answer_rate: number;
  avg_mos?: number;
  avg_loss_rate?: number;
  avg_jitter_ms?: number;
  registered_users: number;
  active_gateways: number;
}

export interface HourlyTrend {
  hour: number;
  total: number;
  answered: number;
}

export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

// ===== 报表 =====
export interface StatusBucket {
  status: string;
  count: number;
  duration_ms: number;
}
export interface DayBucket {
  day: string;
  total: number;
  answered: number;
}
export interface ReportSummary {
  start: string;
  end: string;
  total: number;
  answered: number;
  canceled: number;
  failed: number;
  total_duration_ms: number;
  total_billable_ms: number;
  avg_mos?: number;
  avg_ring_ms?: number;
  avg_setup_ms?: number;
  avg_rtt_ms?: number;
  avg_loss_rate?: number;
  avg_jitter_ms?: number;
  by_status: StatusBucket[];
  by_day: DayBucket[];
}

// ===== 计费 =====
export interface BillingRate {
  id: string;
  prefix: string;
  billing_interval_secs: number;
  price_per_interval: number;
  description?: string;
  created_at?: string;
}
export interface BillingAccount {
  username: string;
  balance: number;
  currency: string;
  created_at?: string;
}
export interface LedgerEntry {
  id: number;
  call_id: string;
  username: string;
  duration_ms: number;
  billing_interval_secs?: number;
  price_per_interval?: number;
  amount: number;
  balance_after: number;
  created_at?: string;
}
export interface ReconcileResult {
  processed: number;
  skipped: number;
  total_amount: number;
}

// ===== 活跃呼叫 =====
export interface ActiveCall {
  call_id: string;
  caller?: string;
  callee?: string;
  state: string;
  started_at_ms: number;
  answered_at_ms?: number;
  gateway?: string;
}

// ===== 号码库存 =====
export interface NumberInventory {
  number: string;
  username?: string;
  gateway_id?: string;
  direction?: string;
  max_concurrent?: number;
  current_concurrent?: number;
  status: string;
  created_at?: string;
  updated_at?: string;
}

// ===== 防盗打 =====
export interface AntiFraudRule {
  id: string;
  rule_type: string;
  target_value: string;
  limit_number: number | null;
  enabled: boolean;
  created_at?: string;
}
export interface AntiFraudConfigItem {
  config_key: string;
  config_value: string;
  description?: string;
  updated_at?: string;
}

export interface AuditLog {
  id: number;
  request_id: string;
  username: string;
  role: string;
  method: string;
  path: string;
  status_code: number;
  source_ip?: string;
  created_at?: string;
}
