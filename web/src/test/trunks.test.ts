import { describe, expect, it } from 'vitest';
import { trunkRole } from '../services/trunks';

describe('trunk role compatibility', () => {
  it('uses the explicit access or egress role', () => {
    expect(trunkRole({ id: 'customer-a', role: 'access' })).toBe('access');
    expect(trunkRole({ id: 'carrier-a', role: 'egress' })).toBe('egress');
  });

  it('maps legacy gateways without exposing legacy labels', () => {
    expect(trunkRole({ id: 'legacy-egress', gateway_type: 'gateway' })).toBe('egress');
    expect(trunkRole({ id: 'legacy-peer', gateway_type: 'peer' })).toBe('access');
  });
});
