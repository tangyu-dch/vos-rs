import { beforeEach, describe, expect, it } from 'vitest';
import { canAccessPage, clearSession, getSession, saveSession } from '@/services/auth';

describe('frontend RBAC', () => {
  beforeEach(() => {
    clearSession();
  });

  it('limits pages by role', () => {
    expect(canAccessPage('admin', '/users')).toBe(true);
    expect(canAccessPage('operator', '/routes')).toBe(true);
    expect(canAccessPage('operator', '/accounts')).toBe(false);
    expect(canAccessPage('financier', '/accounts')).toBe(true);
    expect(canAccessPage('financier', '/gateways')).toBe(false);
  });

  it('persists and validates the login session', () => {
    saveSession({ token: 'token', username: 'alice', role: 'operator' });
    expect(getSession()).toEqual({ token: 'token', username: 'alice', role: 'operator' });

    localStorage.setItem('vos-auth-session', JSON.stringify({ token: '', username: 'alice', role: 'operator' }));
    expect(getSession()).toBeNull();
  });
});
