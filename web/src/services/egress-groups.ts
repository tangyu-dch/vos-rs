import { api } from './client';
import type { Entity } from './resources';

export interface EgressGroup extends Entity {
  id?: string;
  name: string;
  enabled?: boolean;
}

export interface EgressGroupMember extends Entity {
  id?: number;
  group_id: string;
  egress_trunk_id: string;
  destination_prefix?: string;
  priority?: number;
  weight?: number;
  time_start?: string | null;
  time_end?: string | null;
  enabled?: boolean;
  _key?: string; // UI list key
}

export async function getEgressGroup(id: string): Promise<EgressGroup> {
  // api-server endpoints return array or specific object.
  // The endpoints for `/egress-groups/:id` return standard object if loaded properly.
  // Note: /api/v1/egress-groups returns a list, and we can find our group in it,
  // or we can request it if api-server has an endpoint (wait, looking at v1.rs,
  // we only have GET /api/v1/egress-groups, and PUT /api/v1/egress-groups/:id.
  // So there is no GET /api/v1/egress-groups/:id !
  // Let's verify that from v1.rs:
  // .route("/api/v1/egress-groups", get(termination::list_egress_groups).post(termination::create_egress_group))
  // .route("/api/v1/egress-groups/:id", put(termination::update_egress_group).delete(termination::delete_egress_group))
  // Yes! There is no GET /api/v1/egress-groups/:id.
  // So we should fetch all egress groups and find the one we need by ID.
  const list = await api.get<EgressGroup[]>('/egress-groups');
  const found = list.find((item) => String(item.id) === id);
  if (!found) throw new Error(`落地分组 ${id} 不存在`);
  return found;
}

export async function updateEgressGroup(id: string, group: EgressGroup): Promise<void> {
  return api.put<void>(`/egress-groups/${encodeURIComponent(id)}`, group);
}

export async function getEgressGroupMembers(id: string): Promise<EgressGroupMember[]> {
  return api.get<EgressGroupMember[]>(`/egress-groups/${encodeURIComponent(id)}/members`);
}

export async function saveEgressGroupMembers(id: string, members: EgressGroupMember[]): Promise<void> {
  const items = members.map((m) => {
    const item = { ...m };
    delete item._key;
    return item;
  });
  return api.put<void>(`/egress-groups/${encodeURIComponent(id)}/members`, { items });
}
