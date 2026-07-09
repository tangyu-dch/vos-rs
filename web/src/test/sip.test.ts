import { describe, it, expect } from 'vitest';
import { extractSipUser } from '@/utils/sip';

describe('extractSipUser', () => {
  it('extracts user from full SIP URI', () => {
    expect(extractSipUser('sip:1001@example.com')).toBe('1001');
  });

  it('extracts user from URI with port', () => {
    expect(extractSipUser('sip:1001@192.168.1.1:5060')).toBe('1001');
  });

  it('returns plain string if not a SIP URI', () => {
    expect(extractSipUser('1001')).toBe('1001');
  });

  it('handles empty string', () => {
    expect(extractSipUser('')).toBe('—');
  });

  it('extracts user from URI with transport param', () => {
    expect(extractSipUser('sip:user@host:5060;transport=udp')).toBe('user');
  });
});
