export type UserRole = 'admin' | 'operator' | 'financier';

export interface AuthSession {
  token: string;
  username: string;
  role: UserRole;
}

const SESSION_KEY = 'vos-auth-session';

export function getSession(): AuthSession | null {
  const raw = localStorage.getItem(SESSION_KEY);
  if (!raw) return null;

  try {
    const session = JSON.parse(raw) as AuthSession;
    if (!session.token || !session.username || !isUserRole(session.role)) {
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
  return value === 'admin' || value === 'operator' || value === 'financier';
}

export function roleLabel(role: UserRole): string {
  return {
    admin: '管理员',
    operator: '运维',
    financier: '财务',
  }[role];
}

export function canAccessPage(role: UserRole, path: string): boolean {
  if (role === 'admin') return true;
  if (path === '/users') return false;

  if (['/rates', '/accounts'].includes(path)) return role === 'financier';
  if (['/gateways', '/peer-gateways', '/routes', '/numbers', '/anti-fraud'].includes(path)) {
    return role === 'operator';
  }
  if (path === '/audit-logs') return false;

  return [
    '/dashboard',
    '/active-calls',
    '/registrations',
    '/cdr',
    '/reports',
  ].includes(path);
}

export function isForbiddenError(error: unknown): boolean {
  return Boolean(
    typeof error === 'object' &&
      error !== null &&
      (('status' in error && (error as { status?: number }).status === 403) ||
        ('response' in error && (error as { response?: { status?: number } }).response?.status === 403)),
  );
}
