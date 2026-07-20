import { api } from '@/services/client';
import type { Entity } from '@/services/resources';

export interface EgressGroup extends Entity {
  id?: string;
  name: string;
  description?: string;
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

const HH_MM = /^([01]\d|2[0-3]):[0-5]\d$/;
const DESTINATION_PREFIX = /^\+?\d*$/;

export function egressGroupValidationError(group: EgressGroup, members: EgressGroupMember[]): string | null {
  if (!group.name?.trim()) return '分组名称不能为空';
  const seen = new Set<string>();
  for (const member of members) {
    if (!member.egress_trunk_id) return '落地中继不能为空';
    const prefix = member.destination_prefix?.trim() ?? '';
    if (!DESTINATION_PREFIX.test(prefix)) return `落地中继 ${member.egress_trunk_id} 的被叫前缀只能包含数字和开头的 +`;
    if (!Number.isInteger(member.priority) || Number(member.priority) < 0 || Number(member.priority) > 65535) return `落地中继 ${member.egress_trunk_id} 的优先级必须是 0 到 65535 的整数`;
    if (!Number.isInteger(member.weight) || Number(member.weight) < 1 || Number(member.weight) > 10000) return `落地中继 ${member.egress_trunk_id} 的权重必须是 1 到 10000 的整数`;
    const hasStart = Boolean(member.time_start);
    const hasEnd = Boolean(member.time_end);
    if (hasStart !== hasEnd) return '时间窗口必须成对出现';
    if ((member.time_start && !HH_MM.test(member.time_start)) || (member.time_end && !HH_MM.test(member.time_end))) return '时间窗口必须使用 24 小时制 HH:MM';
    const identity = [member.egress_trunk_id, prefix, member.time_start ?? '', member.time_end ?? ''].join('|');
    if (seen.has(identity)) return `落地中继 ${member.egress_trunk_id} 存在重复的匹配规则`;
    seen.add(identity);
  }
  return null;
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
