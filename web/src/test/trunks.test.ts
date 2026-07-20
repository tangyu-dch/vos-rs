import { describe, expect, it } from 'vitest';
import { policyForSave, trunkForSave, trunkRole, trunkValidationError } from '@/services/trunks';

describe('trunk role compatibility', () => {
  it('uses the explicit access or egress role', () => {
    expect(trunkRole({ id: 'customer-a', role: 'access' })).toBe('access');
    expect(trunkRole({ id: 'carrier-a', role: 'egress' })).toBe('egress');
  });

  it('maps legacy gateways without exposing legacy labels', () => {
    expect(trunkRole({ id: 'legacy-egress', gateway_type: 'gateway' })).toBe('egress');
    expect(trunkRole({ id: 'legacy-peer', gateway_type: 'peer' })).toBe('access');
  });

  it('removes mutually exclusive policy fields before saving', () => {
    expect(policyForSave({ caller_mode: 'strict_passthrough', fallback_mode: 'reject', fixed_number: '10086', caller_pool_id: 'pool-a', egress_mode: 'direct', direct_egress_trunk_id: 'carrier-a', egress_group_id: 'group-a' })).toEqual({ caller_mode: 'strict_passthrough', fallback_mode: 'reject', egress_mode: 'direct', direct_egress_trunk_id: 'carrier-a' });
  });

  it('does not send an empty password or response-only fields on edit', () => {
    expect(trunkForSave({ id: 'customer-a', access_password: '', has_access_password: true, current_concurrent: 2 })).toEqual({ id: 'customer-a' });
  });

  it('requires a new password when the digest username changes', () => {
    expect(trunkValidationError(
      { role: 'access', access_auth_mode: 'digest_register', access_username: 'new-user', has_access_password: true },
      { role: 'access', access_username: 'old-user', has_access_password: true },
      [],
      [],
    )).toBe('修改注册用户后必须重新输入注册密码');
  });

  it('requires an enabled IP row and validates entered egress endpoints', () => {
    expect(trunkValidationError(
      { role: 'access', access_auth_mode: 'ip_allowlist' },
      {},
      [{ cidr: '192.0.2.10/32', transport: 'udp', enabled: false }],
      [],
    )).toBe('请至少配置一条已启用且完整的 IP 白名单');
    expect(trunkValidationError({ role: 'egress' }, {}, [], [{ host: '', transport: 'udp', enabled: false }]))
      .toBe('落地端点的主机地址不能为空');
  });

  it('validates egress trunk registration when connection type is client_register', () => {
    expect(trunkValidationError(
      { role: 'egress', egress_connection_type: 'client_register' },
      {},
      [],
      [],
    )).toBe('注册服务器不能为空');

    expect(trunkValidationError(
      { role: 'egress', egress_connection_type: 'client_register', register_server: 'sip.carrier.com' },
      {},
      [],
      [],
    )).toBe('注册用户不能为空');

    expect(trunkValidationError(
      { role: 'egress', egress_connection_type: 'client_register', register_server: 'sip.carrier.com', register_username: 'user1' },
      {},
      [],
      [],
    )).toBe('注册认证必须填写注册密码');

    expect(trunkValidationError(
      { role: 'egress', egress_connection_type: 'client_register', register_server: 'sip.carrier.com', register_username: 'user2' },
      { reg_username: 'user1', has_register_password: true },
      [],
      [],
    )).toBe('修改注册用户后必须重新输入注册密码');

    expect(trunkValidationError(
      { role: 'egress', egress_connection_type: 'client_register', register_server: 'sip.carrier.com', register_username: 'user2', register_password: 'pwd' },
      { reg_username: 'user1', has_register_password: true },
      [],
      [],
    )).toBeNull();
  });
});
