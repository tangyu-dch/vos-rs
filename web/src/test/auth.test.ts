import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  canAccessPage,
  clearSession,
  firstMenuPath,
  getSession,
  hasPermission,
  saveSession,
  type AuthSession,
} from '@/services/auth';
import { api } from '@/services/client';
import { login } from '@/services/resources';

describe('frontend RBAC', () => {
  beforeEach(() => {
    clearSession();
  });

  const session: AuthSession = {
    token: 'token',
    username: 'alice',
    display_name: 'Alice',
    role: 'custom',
    role_name: '自定义角色',
    permissions: ['calls.view'],
    menus: [
      {
        group_key: 'analytics',
        label: '通话分析',
        icon_key: 'phone',
        sort_order: 1,
        enabled: true,
        items: [
          {
            item_key: 'calls',
            label: '通话记录',
            path: '/calls',
            icon_key: 'phone',
            permission_key: 'calls.view',
            sort_order: 1,
            enabled: true,
          },
        ],
      },
    ],
  };

  it('uses database menus to limit pages', () => {
    expect(canAccessPage(session, '/calls')).toBe(true);
    expect(canAccessPage(session, '/calls/example')).toBe(false);
    expect(canAccessPage(session, '/billing/accounts')).toBe(false);
  });

  it('uses the first enabled database menu as the home page', () => {
    expect(firstMenuPath(session)).toBe('/calls');
  });

  it('does not treat a sibling button permission as authorization', () => {
    const operator = { ...session, permissions: ['calls.monitor'] };
    expect(hasPermission(operator, 'calls.monitor')).toBe(true);
    expect(hasPermission(operator, 'calls.terminate')).toBe(false);
    expect(hasPermission(operator, 'calls.play')).toBe(false);
  });

  it('wildcard permission authorizes every button action', () => {
    expect(hasPermission({ ...session, permissions: ['*'] }, 'infrastructure.manage')).toBe(true);
  });

  it('persists and validates the login session', () => {
    saveSession(session);
    expect(getSession()).toEqual(session);

    localStorage.setItem(
      'vos-auth-session',
      JSON.stringify({ token: '', username: 'alice', role: 'operator' }),
    );
    expect(getSession()).toBeNull();
  });

  it('does not create an administrator session when authentication fails', async () => {
    vi.spyOn(api, 'post').mockRejectedValueOnce(new Error('service unavailable'));

    await expect(login('admin', 'admin')).rejects.toThrow('service unavailable');
    expect(getSession()).toBeNull();
  });
});
