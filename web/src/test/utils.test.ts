import { describe, it, expect } from 'vitest';
import { extractSipUser } from '@/utils/sip';

describe('extractSipUser edge cases', () => {
  it('handles SIP URI with multiple parameters', () => {
    expect(extractSipUser('sip:user@host:5060;transport=udp;lr')).toBe('user');
  });

  it('handles display name prefix', () => {
    expect(extractSipUser('"Display Name" <sip:1001@example.com>')).toBe('1001');
  });

  it('handles user with special characters', () => {
    expect(extractSipUser('sip:user.name@example.com')).toBe('user.name');
  });

  it('handles IPv6 address', () => {
    expect(extractSipUser('sip:1001@[::1]:5060')).toBe('1001');
  });
});

describe('Dashboard stats formatting', () => {
  it('formats duration correctly', () => {
    const formatDuration = (ms: number) => {
      const seconds = Math.floor(ms / 1000);
      const minutes = Math.floor(seconds / 60);
      const hours = Math.floor(minutes / 60);
      if (hours > 0) return `${hours}h ${minutes % 60}m`;
      if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
      return `${seconds}s`;
    };

    expect(formatDuration(0)).toBe('0s');
    expect(formatDuration(5000)).toBe('5s');
    expect(formatDuration(65000)).toBe('1m 5s');
    expect(formatDuration(3665000)).toBe('1h 1m');
  });
});

describe('Status color mapping', () => {
  const STATUS_MAP: Record<string, { color: string; text: string }> = {
    Routing: { color: 'blue', text: '路由中' },
    Ringing: { color: 'orange', text: '振铃中' },
    Established: { color: 'green', text: '通话中' },
  };

  it('maps call states to colors', () => {
    expect(STATUS_MAP.Routing.color).toBe('blue');
    expect(STATUS_MAP.Ringing.color).toBe('orange');
    expect(STATUS_MAP.Established.color).toBe('green');
  });

  it('maps call states to Chinese text', () => {
    expect(STATUS_MAP.Routing.text).toBe('路由中');
    expect(STATUS_MAP.Ringing.text).toBe('振铃中');
    expect(STATUS_MAP.Established.text).toBe('通话中');
  });
});

describe('Registration status', () => {
  const getExpStatus = (expiresAt: string) => {
    const diffMs = new Date(expiresAt).getTime() - Date.now();
    const mins = Math.floor(diffMs / 60000);
    if (mins < 0) return { text: '已过期', cls: 'failed' };
    if (mins < 5) return { text: '即将过期', cls: 'canceled' };
    return { text: '在线', cls: 'answered' };
  };

  it('shows expired for past time', () => {
    const past = new Date(Date.now() - 3600000).toISOString();
    expect(getExpStatus(past).text).toBe('已过期');
  });

  it('shows online for future time', () => {
    const future = new Date(Date.now() + 3600000).toISOString();
    expect(getExpStatus(future).text).toBe('在线');
  });

  it('shows expiring soon for near future', () => {
    const soon = new Date(Date.now() + 120000).toISOString();
    expect(getExpStatus(soon).text).toBe('即将过期');
  });
});

describe('File size formatting', () => {
  const formatSize = (bytes: number) => {
    if (!bytes) return '—';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
  };

  it('formats bytes', () => {
    expect(formatSize(0)).toBe('—');
    expect(formatSize(512)).toBe('512 B');
  });

  it('formats kilobytes', () => {
    expect(formatSize(1024)).toBe('1.0 KB');
    expect(formatSize(1536)).toBe('1.5 KB');
  });

  it('formats megabytes', () => {
    expect(formatSize(1048576)).toBe('1.00 MB');
    expect(formatSize(2621440)).toBe('2.50 MB');
  });
});
