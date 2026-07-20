import { api } from '@/services/client';
import type { Entity } from '@/services/resources';

export interface CallerPool extends Entity {
  id?: string;
  owner_source_type: 'trunk' | 'extension' | 'extension_group';
  owner_source_id: string;
  virtual_alias: string;
  strategy: 'random' | 'round_robin' | 'weighted_random' | 'stable_hash';
  fallback_mode: 'reject' | 'fixed' | 'pool';
  enabled?: boolean;
}

export interface CallerPoolMember extends Entity {
  id?: number;
  pool_id: string;
  number: string;
  priority?: number;
  weight?: number;
  max_concurrent?: number;
  enabled?: boolean;
  _key?: string; // UI list key
}

export function callerPoolValidationError(pool: CallerPool, members: CallerPoolMember[]): string | null {
  if (!pool.virtual_alias?.trim() || !pool.owner_source_id?.trim()) return '虚拟主叫与来源标识不能为空';
  const seen = new Set<string>();
  for (const member of members) {
    const number = member.number?.trim();
    if (!number) return '号码不能为空';
    if (seen.has(number)) return `真实号码 ${number} 不能重复加入同一号码池`;
    seen.add(number);
    if (!Number.isInteger(member.priority) || Number(member.priority) < 0 || Number(member.priority) > 65535) return `真实号码 ${number} 的优先级必须是 0 到 65535 的整数`;
    if (!Number.isInteger(member.weight) || Number(member.weight) < 1 || Number(member.weight) > 10000) return `真实号码 ${number} 的权重必须是 1 到 10000 的整数`;
    if (!Number.isInteger(member.max_concurrent) || Number(member.max_concurrent) < 0) return `真实号码 ${number} 的最大并发必须是非负整数`;
  }
  return null;
}

export function numberCanJoinCallerPool(number: Entity, pool: CallerPool): boolean {
  const direction = String(number.direction ?? 'both');
  const canPresent = number.can_present === undefined
    ? ['outbound', 'both', 'bidirectional'].includes(direction)
    : Boolean(number.can_present);
  return number.status !== 'disabled'
    && canPresent
    && number.allocation_source_type === pool.owner_source_type
    && String(number.allocation_source_id ?? '') === pool.owner_source_id;
}

export async function getCallerPool(id: string): Promise<CallerPool> {
  // Similar to egress groups, v1.rs doesn't expose GET /caller-pools/:id.
  // We fetch all caller pools and find the one we need by ID.
  const list = await api.get<CallerPool[]>('/caller-pools');
  const found = list.find((item) => String(item.id) === id);
  if (!found) throw new Error(`号码池 ${id} 不存在`);
  return found;
}

export async function updateCallerPool(id: string, pool: CallerPool): Promise<void> {
  return api.put<void>(`/caller-pools/${encodeURIComponent(id)}`, { ...pool, fallback_mode: 'reject' });
}

export async function getCallerPoolMembers(id: string): Promise<CallerPoolMember[]> {
  return api.get<CallerPoolMember[]>(`/caller-pools/${encodeURIComponent(id)}/members`);
}

export async function saveCallerPoolMembers(id: string, members: CallerPoolMember[]): Promise<void> {
  const items = members.map((m) => {
    const item = { ...m };
    delete item._key;
    return item;
  });
  return api.put<void>(`/caller-pools/${encodeURIComponent(id)}/members`, { items });
}
