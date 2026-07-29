import { api } from '@/services/client';
import type { MenuGroup } from '@/services/auth';

export interface ConsoleUser {
  username: string;
  display_name: string;
  role_key: string;
  role_name: string;
  enabled: boolean;
  is_builtin: boolean;
}

export interface AccessRole {
  role_key: string;
  name: string;
  description: string;
  is_system: boolean;
  enabled: boolean;
  permission_keys: string[];
}

export interface AccessPermission {
  permission_key: string;
  name: string;
  group_name: string;
  description: string;
}

export interface AccessOverview {
  users: ConsoleUser[];
  roles: AccessRole[];
  permissions: AccessPermission[];
  menus: MenuGroup[];
}

export const getAccountOverview = () => api.get<AccessOverview>('/access-control/accounts');
export const getRoleOverview = () => api.get<AccessOverview>('/access-control/role-permissions');
export const createConsoleUser = (body: {
  username: string;
  password: string;
  display_name: string;
  role_key: string;
}) => api.post<ConsoleUser>('/access-control/users', body);
export const updateConsoleUser = (
  username: string,
  body: { display_name: string; role_key: string; enabled: boolean; password?: string },
) => api.put<ConsoleUser>(`/access-control/users/${encodeURIComponent(username)}`, body);
export const deleteConsoleUser = (username: string) =>
  api.delete<void>(`/access-control/users/${encodeURIComponent(username)}`);
export const createAccessRole = (body: { role_key: string; name: string; description: string }) =>
  api.post<AccessRole>('/access-control/roles', body);
export const updateAccessRole = (
  roleKey: string,
  body: { name: string; description: string; enabled: boolean },
) => api.put<AccessRole>(`/access-control/roles/${encodeURIComponent(roleKey)}`, body);
export const deleteAccessRole = (roleKey: string) =>
  api.delete<void>(`/access-control/roles/${encodeURIComponent(roleKey)}`);
export const assignUserRoles = (assignments: { username: string; role_key: string }[]) =>
  api.put<void>('/access-control/roles/user-assignments', { assignments });
export const replaceRolePermissions = (roleKey: string, permissionKeys: string[]) =>
  api.put<AccessRole>(`/access-control/roles/${encodeURIComponent(roleKey)}/permissions`, {
    permission_keys: permissionKeys,
  });
