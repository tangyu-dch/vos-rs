import { describe, expect, it } from 'vitest';
import { policyForSave, trunkRole } from '../services/trunks';

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
});
