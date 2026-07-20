import { api, type PageResult } from '@/services/client';
import type { Entity } from '@/services/resources';

export type TrunkRole = 'access' | 'egress';
export type AccessAuthType = 'ip_allowlist' | 'digest_register' | 'ip_and_digest';
export type EgressConnectionType = 'static_peer' | 'client_register';
export type CallerPolicyMode = 'strict_passthrough' | 'fixed_number' | 'virtual_pool';

export interface TrunkIpRule extends Entity {
  _key?: string;
  id?: string;
  cidr: string;
  source_port?: number | null;
  transport: 'udp';
  description?: string;
  enabled: boolean;
}

export interface EgressEndpoint extends Entity {
  _key?: string;
  id?: string;
  host: string;
  port?: number | null;
  transport: 'udp';
  priority?: number;
  enabled: boolean;
}

export interface OutboundPolicy extends Entity {
  caller_mode: CallerPolicyMode;
  fallback_mode: 'reject' | 'fixed' | 'pool';
  fixed_number?: string;
  caller_pool_id?: string;
  egress_mode: 'direct' | 'group';
  direct_egress_trunk_id?: string;
  egress_group_id?: string;
  enabled?: boolean;
}

export interface TrunkWorkspaceData {
  trunk: Entity;
  health?: Entity;
  registrations?: Entity[];
  numbers?: Entity[];
}

export function trunkRole(trunk: Entity): TrunkRole {
  if (trunk.role === 'access' || trunk.role === 'egress') return trunk.role;
  return trunk.gateway_type === 'gateway' ? 'egress' : 'access';
}

export async function getTrunkWorkspace(id: string): Promise<TrunkWorkspaceData> {
  const result = await api.get<Entity>(`/trunks/${encodeURIComponent(id)}`);
  const trunk = result.trunk && typeof result.trunk === 'object' ? result.trunk as Entity : result;
  return {
    trunk,
    health: result.health as Entity | undefined,
    registrations: Array.isArray(result.registrations) ? result.registrations as Entity[] : [],
    numbers: Array.isArray(result.numbers) ? result.numbers as Entity[] : [],
  };
}

export function updateTrunk(id: string, values: Entity) {
  return api.put<Entity>(`/trunks/${encodeURIComponent(id)}`, trunkForSave(values));
}

export function trunkForSave(values: Entity): Entity {
  const body = { ...values };
  if (typeof body.access_password === 'string' && !body.access_password.trim()) {
    delete body.access_password;
  }
  delete body.has_access_password;
  delete body.has_register_password;
  delete body.egress_connection_type;
  delete body.register_server;
  delete body.register_username;
  delete body.register_password;
  delete body.created_at;
  delete body.current_concurrent;
  delete body.circuit_state;
  return body;
}

export function trunkValidationError(
  draft: Entity,
  original: Entity,
  rules: TrunkIpRule[],
  endpoints: EgressEndpoint[],
): string | null {
  const role = trunkRole(draft);
  if (role === 'access') {
    const mode = String(draft.access_auth_mode ?? '');
    const needsIp = mode === 'ip_allowlist' || mode === 'ip_and_digest';
    const needsDigest = mode === 'digest_register' || mode === 'ip_and_digest';
    if (!['ip_allowlist', 'digest_register', 'ip_and_digest'].includes(mode)) return '请选择接入认证方式';
    if (rules.some((rule) => !rule.cidr.trim())) return 'IP 白名单地址不能为空';
    if (needsIp && !rules.some((rule) => rule.enabled)) return '请至少配置一条已启用且完整的 IP 白名单';
    if (needsDigest) {
      const username = String(draft.access_username ?? '').trim();
      const password = String(draft.access_password ?? '');
      if (!username) return '注册认证必须填写注册用户';
      const usernameChanged = username !== String(original.access_username ?? '').trim();
      if ((!original.has_access_password || usernameChanged) && !password) {
        return usernameChanged ? '修改注册用户后必须重新输入注册密码' : '注册认证必须填写注册密码';
      }
    }
  }
  if (role === 'egress') {
    const connectionType = String(draft.egress_connection_type ?? 'static_peer');
    if (connectionType === 'client_register') {
      const server = String(draft.register_server ?? '').trim();
      const username = String(draft.register_username ?? '').trim();
      const password = String(draft.register_password ?? '');
      if (!server) return '注册服务器不能为空';
      if (!username) return '注册用户不能为空';
      const usernameChanged = original.reg_username ? (username !== String(original.reg_username).trim()) : false;
      if ((!original.has_register_password || usernameChanged) && !password) {
        return usernameChanged ? '修改注册用户后必须重新输入注册密码' : '注册认证必须填写注册密码';
      }
    } else {
      if (endpoints.some((endpoint) => !endpoint.host.trim())) return '落地端点的主机地址不能为空';
    }
  }
  return null;
}

export function getTrunkIpRules(id: string) {
  return api.get<TrunkIpRule[]>(`/trunks/${encodeURIComponent(id)}/ip-rules`);
}

export function saveTrunkIpRules(id: string, rules: TrunkIpRule[]) {
  const items = rules.map((rule) => { const item = { ...rule }; delete item._key; return item; });
  return api.put<TrunkIpRule[]>(`/trunks/${encodeURIComponent(id)}/ip-rules`, { items });
}

export function getTrunkEgressEndpoints(id: string) {
  return api.get<EgressEndpoint[]>(`/trunks/${encodeURIComponent(id)}/egress-endpoints`);
}

export function saveTrunkEgressEndpoints(id: string, endpoints: EgressEndpoint[]) {
  const items = endpoints.map((ep) => { const item = { ...ep }; delete item._key; return item; });
  return api.put<EgressEndpoint[]>(`/trunks/${encodeURIComponent(id)}/egress-endpoints`, { items });
}

export function getOutboundPolicy(id: string) {
  return api.get<OutboundPolicy>(`/trunks/${encodeURIComponent(id)}/outbound-policy`);
}

export function saveOutboundPolicy(id: string, policy: OutboundPolicy) {
  return api.put<OutboundPolicy>(`/trunks/${encodeURIComponent(id)}/outbound-policy`, policyForSave(policy));
}

export function policyForSave(policy: OutboundPolicy): OutboundPolicy {
  const body = { ...policy };
  body.fallback_mode = 'reject';
  if (body.caller_mode !== 'fixed_number') delete body.fixed_number;
  if (body.caller_mode !== 'virtual_pool') delete body.caller_pool_id;
  if (body.egress_mode === 'direct') delete body.egress_group_id;
  else delete body.direct_egress_trunk_id;
  return body;
}

export function policyValidationError(policy: OutboundPolicy): string | null {
  if (policy.caller_mode === 'fixed_number' && !policy.fixed_number?.trim()) return '固定号码策略必须选择真实号码';
  if (policy.caller_mode === 'virtual_pool' && !policy.caller_pool_id?.trim()) return '虚拟主叫策略必须选择号码池';
  if (policy.egress_mode === 'direct' && !policy.direct_egress_trunk_id?.trim()) return '请选择直接绑定的落地中继';
  if (policy.egress_mode === 'group' && !policy.egress_group_id?.trim()) return '请选择允许使用的落地分组';
  return null;
}

export async function listOptions(path: '/caller-pools' | '/egress-groups' | '/trunks' | '/numbers' | '/extensions'): Promise<Entity[]> {
  const result = await api.get<PageResult<Entity> | Entity[]>(path, { page: 1, page_size: 200 });
  return Array.isArray(result) ? result : result.items || [];
}
