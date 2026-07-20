import { describe, expect, it } from 'vitest';
import {
  callerPoolValidationError,
  numberCanJoinCallerPool,
  type CallerPool,
  type CallerPoolMember,
} from '@/services/caller-pools';
import {
  egressGroupValidationError,
  type EgressGroup,
  type EgressGroupMember,
} from '@/services/egress-groups';

const pool: CallerPool = {
  id: 'pool-a',
  owner_source_type: 'trunk',
  owner_source_id: 'access-a',
  virtual_alias: '销售热线',
  strategy: 'weighted_random',
  fallback_mode: 'reject',
  enabled: true,
};

const member: CallerPoolMember = {
  pool_id: 'pool-a',
  number: '10086',
  priority: 100,
  weight: 100,
  max_concurrent: 1,
  enabled: true,
};

describe('caller pool configuration', () => {
  it('only offers an allocated number that can be presented', () => {
    expect(numberCanJoinCallerPool({
      number: '10086',
      allocation_source_type: 'trunk',
      allocation_source_id: 'access-a',
      direction: 'both',
      status: 'assigned',
    }, pool)).toBe(true);
    expect(numberCanJoinCallerPool({
      number: '10010',
      allocation_source_type: 'trunk',
      allocation_source_id: 'access-b',
      direction: 'both',
      status: 'assigned',
    }, pool)).toBe(false);
    expect(numberCanJoinCallerPool({
      number: '10000',
      allocation_source_type: 'trunk',
      allocation_source_id: 'access-a',
      direction: 'inbound',
      status: 'assigned',
    }, pool)).toBe(false);
  });

  it('rejects duplicate numbers and invalid numeric limits', () => {
    expect(callerPoolValidationError(pool, [member, { ...member }])).toContain('不能重复');
    expect(callerPoolValidationError(pool, [{ ...member, weight: 0 }])).toContain('权重');
    expect(callerPoolValidationError(pool, [{ ...member, max_concurrent: -1 }])).toContain('最大并发');
    expect(callerPoolValidationError(pool, [member])).toBeNull();
  });
});

const group: EgressGroup = { id: 'group-a', name: '默认落地', enabled: true };
const groupMember: EgressGroupMember = {
  group_id: 'group-a',
  egress_trunk_id: 'carrier-a',
  destination_prefix: '86',
  priority: 100,
  weight: 100,
  time_start: '08:00',
  time_end: '22:00',
  enabled: true,
};

describe('egress group configuration', () => {
  it('validates time windows and destination prefixes', () => {
    expect(egressGroupValidationError(group, [{ ...groupMember, time_start: '24:00' }])).toContain('HH:MM');
    expect(egressGroupValidationError(group, [{ ...groupMember, time_end: null }])).toContain('成对');
    expect(egressGroupValidationError(group, [{ ...groupMember, destination_prefix: '86*' }])).toContain('被叫前缀');
    expect(egressGroupValidationError(group, [groupMember])).toBeNull();
  });

  it('rejects duplicate match rules and zero weight', () => {
    expect(egressGroupValidationError(group, [groupMember, { ...groupMember }])).toContain('重复');
    expect(egressGroupValidationError(group, [{ ...groupMember, weight: 0 }])).toContain('权重');
  });
});
