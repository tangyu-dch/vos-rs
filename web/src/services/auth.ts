export type UserRole = string;

export interface MenuItem {
  item_key: string;
  label: string;
  path: string;
  icon_key: string;
  permission_key: string;
  sort_order: number;
  enabled: boolean;
}

export interface MenuGroup {
  group_key: string;
  label: string;
  icon_key: string;
  sort_order: number;
  enabled: boolean;
  items: MenuItem[];
}

export interface AuthSession {
  token: string;
  username: string;
  display_name: string;
  role: UserRole;
  role_name: string;
  permissions: string[];
  menus: MenuGroup[];
}

const SESSION_KEY = 'vos-auth-session';

export function getSession(): AuthSession | null {
  const raw = localStorage.getItem(SESSION_KEY);
  if (!raw) return null;
  try {
    const session = JSON.parse(raw) as AuthSession;
    if (
      !session.token ||
      !session.username ||
      !session.role ||
      !Array.isArray(session.permissions) ||
      !Array.isArray(session.menus)
    ) {
      clearSession();
      return null;
    }
    return session;
  } catch {
    clearSession();
    return null;
  }
}

export function saveSession(session: AuthSession): void {
  localStorage.setItem(SESSION_KEY, JSON.stringify(session));
}

export function clearSession(): void {
  localStorage.removeItem(SESSION_KEY);
}

export function getAccessToken(): string | null {
  return getSession()?.token ?? null;
}

export function isUserRole(value: string): value is UserRole {
  return value.trim().length > 0;
}

export function roleLabel(role: UserRole, configuredName?: string): string {
  return configuredName || role;
}

export function hasPermission(session: AuthSession, permission: string): boolean {
  return session.permissions.includes('*') || session.permissions.includes(permission);
}

export function canAccessPage(session: AuthSession, path: string): boolean {
  return session.menus.some(
    (group) => group.enabled && group.items.some((item) => item.enabled && path === item.path),
  );
}

export function firstMenuPath(session: AuthSession): string {
  return (
    session.menus
      .filter((group) => group.enabled)
      .sort((left, right) => left.sort_order - right.sort_order)
      .flatMap((group) =>
        group.items
          .filter((item) => item.enabled)
          .sort((left, right) => left.sort_order - right.sort_order),
      )[0]?.path ?? '/login'
  );
}

export function canWriteDomain(
  session: AuthSession,
  domain: 'extensions' | 'operations' | 'billing' | 'system',
): boolean {
  const prefixes: Record<typeof domain, string[]> = {
    extensions: ['extensions.create', 'extensions.update', 'extensions.delete'],
    operations: ['calls.terminate', 'calls.play', 'calls.mute', 'calls.monitor'],
    billing: [
      'billing.accounts.credit',
      'billing.rates.create',
      'billing.rates.update',
      'billing.rates.delete',
    ],
    system: [
      'settings.manage',
      'infrastructure.manage',
      'access.users',
      'access.roles',
      'llm.manage',
      'llm.create',
      'llm.update',
      'llm.delete',
      'llm.activate',
      'access.accounts.create',
      'access.accounts.update',
      'access.accounts.delete',
      'access.roles.create',
      'access.roles.update',
      'access.roles.delete',
      'access.roles.permissions',
      'access.roles.assign',
    ],
  };
  return prefixes[domain].some((permission) => hasPermission(session, permission));
}

export function isForbiddenError(error: unknown): boolean {
  return Boolean(
    typeof error === 'object' &&
    error !== null &&
    (('status' in error && (error as { status?: number }).status === 403) ||
      ('response' in error &&
        (error as { response?: { status?: number } }).response?.status === 403)),
  );
}
