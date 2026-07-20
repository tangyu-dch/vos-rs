import { api } from '@/services/client';
import type { Entity } from '@/services/resources';
import { policyForSave, type OutboundPolicy } from '@/services/trunks';

export interface ExtensionWorkspaceData {
  extension: Entity;
  registrations: Entity[];
  numbers: Entity[];
  credential?: Entity;
}

export async function getExtensionWorkspace(username: string): Promise<ExtensionWorkspaceData> {
  const result = await api.get<Entity>(`/extensions/${encodeURIComponent(username)}`);
  return {
    extension: result.extension && typeof result.extension === 'object' ? result.extension as Entity : result,
    registrations: Array.isArray(result.registrations) ? result.registrations as Entity[] : [],
    numbers: Array.isArray(result.numbers) ? result.numbers as Entity[] : [],
    credential: result.credential as Entity | undefined,
  };
}

export function updateExtensionPassword(username: string, password: string) {
  return api.put<void>(`/extensions/${encodeURIComponent(username)}`, { password });
}

export function getExtensionOutboundPolicy(username: string) {
  return api.get<OutboundPolicy>(`/extensions/${encodeURIComponent(username)}/outbound-policy`);
}

export function saveExtensionOutboundPolicy(username: string, policy: OutboundPolicy) {
  return api.put<void>(`/extensions/${encodeURIComponent(username)}/outbound-policy`, policyForSave(policy));
}
