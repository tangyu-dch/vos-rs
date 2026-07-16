import { api, type PageResult } from './client';
import type { Entity } from './resources';

export type TrunkRole = 'access' | 'egress';
export type AccessAuthType = 'ip_allowlist' | 'digest_register' | 'ip_and_digest';
export type EgressConnectionType = 'static_peer' | 'client_register';
export type CallerPolicyMode = 'strict_passthrough' | 'fixed_number' | 'virtual_pool';

export interface TrunkIpRule extends Entity {
  id?: string;
  cidr: string;
  source_port?: number | null;
  transport: 'udp';
  description?: string;
  enabled: boolean;
}

export interface OutboundPolicy extends Entity {
  caller_policy: CallerPolicyMode;
  caller_failure_action: 'reject' | 'fallback_number' | 'fallback_pool';
  fixed_number_id?: string;
  caller_pool_id?: string;
  fallback_number_id?: string;
  fallback_pool_id?: string;
  binding_type: 'direct_trunk' | 'egress_group';
  egress_trunk_id?: string;
  egress_group_id?: string;
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
  return api.put<Entity>(`/trunks/${encodeURIComponent(id)}`, values);
}

export function getTrunkIpRules(id: string) {
  return api.get<TrunkIpRule[]>(`/trunks/${encodeURIComponent(id)}/ip-rules`);
}

export function saveTrunkIpRules(id: string, rules: TrunkIpRule[]) {
  return api.put<TrunkIpRule[]>(`/trunks/${encodeURIComponent(id)}/ip-rules`, { items: rules });
}

export function getOutboundPolicy(id: string) {
  return api.get<OutboundPolicy>(`/trunks/${encodeURIComponent(id)}/outbound-policy`);
}

export function saveOutboundPolicy(id: string, policy: OutboundPolicy) {
  return api.put<OutboundPolicy>(`/trunks/${encodeURIComponent(id)}/outbound-policy`, policy);
}

export async function listOptions(path: '/caller-pools' | '/egress-groups' | '/trunks'): Promise<Entity[]> {
  const result = await api.get<PageResult<Entity> | Entity[]>(path, { page: 1, page_size: 200 });
  return Array.isArray(result) ? result : result.items || [];
}
