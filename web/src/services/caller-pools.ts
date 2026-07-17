import { api } from './client';
import type { Entity } from './resources';

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

export async function getCallerPool(id: string): Promise<CallerPool> {
  // Similar to egress groups, v1.rs doesn't expose GET /caller-pools/:id.
  // We fetch all caller pools and find the one we need by ID.
  const list = await api.get<CallerPool[]>('/caller-pools');
  const found = list.find((item) => String(item.id) === id);
  if (!found) throw new Error(`号码池 ${id} 不存在`);
  return found;
}

export async function updateCallerPool(id: string, pool: CallerPool): Promise<void> {
  return api.put<void>(`/caller-pools/${encodeURIComponent(id)}`, pool);
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
