import { describe, expect, it } from 'vitest';
import { permissionKeysForMenu } from '@/pages/system/role-permissions';

describe('角色权限页面映射', () => {
  it('呼入目标使用落地管理权限', () => {
    expect(permissionKeysForMenu('did', 'numbers.view')).toEqual([
      'termination.view',
      'termination.manage',
    ]);
  });

  it('活跃通话与历史话单只展示页面实际按钮', () => {
    expect(permissionKeysForMenu('active_calls', 'calls.view')).toEqual([
      'calls.view',
      'calls.export',
      'calls.terminate',
    ]);
    expect(permissionKeysForMenu('calls', 'calls.view')).toEqual(['calls.view', 'calls.export']);
  });

  it('未单独配置的菜单回退到菜单访问权限', () => {
    expect(permissionKeysForMenu('custom', 'custom.view')).toEqual(['custom.view']);
  });

  it('权限管理页面按查看和按钮动作拆分', () => {
    expect(permissionKeysForMenu('access_control', 'access.accounts.view')).toEqual([
      'access.accounts.view',
      'access.accounts.create',
      'access.accounts.update',
      'access.accounts.delete',
    ]);
    expect(permissionKeysForMenu('role_permissions', 'access.roles.view')).toEqual([
      'access.roles.view',
      'access.roles.create',
      'access.roles.update',
      'access.roles.delete',
      'access.roles.permissions',
      'access.roles.assign',
    ]);
  });

  it('模型配置按增删改和启用动作拆分', () => {
    expect(permissionKeysForMenu('llm', 'llm.view')).toEqual([
      'llm.view',
      'llm.create',
      'llm.update',
      'llm.delete',
      'llm.activate',
    ]);
  });
});
