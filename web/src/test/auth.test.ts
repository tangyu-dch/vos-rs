import { beforeEach, describe, expect, it, vi } from 'vitest';
import { canAccessPage, clearSession, getSession, saveSession } from '@/services/auth';
import { api } from '@/services/client';
import { login } from '@/services/resources';

describe('frontend RBAC', () => {
  beforeEach(() => {
    clearSession();
  });

  it('limits pages by role', () => {
    expect(canAccessPage('admin', '/extensions')).toBe(true);
    expect(canAccessPage('operator', '/extensions')).toBe(false);
    expect(canAccessPage('operator', '/routing')).toBe(true);
    expect(canAccessPage('operator', '/billing/accounts')).toBe(false);
    expect(canAccessPage('financier', '/billing/accounts')).toBe(true);
    expect(canAccessPage('financier', '/trunks')).toBe(false);
    expect(canAccessPage('operator', '/infrastructure')).toBe(false);
    expect(canAccessPage('financier', '/calls/example')).toBe(true);
    expect(canAccessPage('operator', '/settings')).toBe(false);
    expect(canAccessPage('financier', '/settings')).toBe(false);
  });

  it('persists and validates the login session', () => {
    saveSession({ token: 'token', username: 'alice', role: 'operator' });
    expect(getSession()).toEqual({ token: 'token', username: 'alice', role: 'operator' });

    localStorage.setItem('vos-auth-session', JSON.stringify({ token: '', username: 'alice', role: 'operator' }));
    expect(getSession()).toBeNull();
  });

  it('does not create an administrator session when authentication fails', async () => {
    vi.spyOn(api, 'post').mockRejectedValueOnce(new Error('service unavailable'));

    await expect(login('admin', 'admin')).rejects.toThrow('service unavailable');
    expect(getSession()).toBeNull();
  });
});
