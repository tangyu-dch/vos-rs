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
