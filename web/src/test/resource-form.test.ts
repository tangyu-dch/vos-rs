import { describe, expect, it } from 'vitest';
import { resourceFormValues, resourceSaveValues } from '@/pages/shared/resource-workspace';

const routeSpec = {
  title: '路由策略',
  description: '',
  path: '/routing/rules',
  idKey: 'id',
  fields: [
    { key: 'id', label: '规则 ID', required: true },
    { key: 'priority', label: '优先级', kind: 'number' as const, defaultValue: 100 },
    { key: 'enabled', label: '启用', kind: 'switch' as const, defaultValue: true },
  ],
};

describe('resource form values', () => {
  it('copies every resource value into an edit form', () => {
    const row = { id: 'default', prefix: '86', priority: 200, time_start: '08:00' };
    expect(resourceFormValues(routeSpec, row)).toEqual(row);
    expect(resourceFormValues(routeSpec, row)).not.toBe(row);
  });

  it('applies configured defaults only to a create form', () => {
    expect(resourceFormValues(routeSpec, null)).toEqual({ priority: 100, enabled: true });
  });

  it('omits an empty secret when editing so the stored password is preserved', () => {
    const trunkSpec = {
      ...routeSpec,
      idKey: 'id',
      fields: [...routeSpec.fields, { key: 'reg_password', label: '注册密码', kind: 'secret' as const, preserveEmptyOnEdit: true }],
    };
    expect(resourceSaveValues(trunkSpec, { id: 'carrier', host: '127.0.0.1', reg_password: '' }, true))
      .toEqual({ id: 'carrier', host: '127.0.0.1' });
    expect(resourceSaveValues(trunkSpec, { id: 'carrier', reg_password: 'new-secret' }, true))
      .toEqual({ id: 'carrier', reg_password: 'new-secret' });
  });

  it('keeps required secrets that the update endpoint cannot preserve', () => {
    const extensionSpec = {
      ...routeSpec,
      idKey: 'username',
      fields: [{ key: 'username', label: '分机号' }, { key: 'password', label: 'SIP 密码', kind: 'secret' as const, required: true }],
    };
    expect(resourceSaveValues(extensionSpec, { username: '1001', password: '' }, true))
      .toEqual({ username: '1001', password: '' });
  });
});
