import { describe, expect, it, vi } from 'vitest';

vi.mock('axios', () => ({
  default: {
    create: vi.fn(() => ({
      request: vi.fn(),
      interceptors: {
        request: { use: vi.fn() },
        response: { use: vi.fn() },
      },
    })),
  },
}));

import { ApiError, shouldRetryRequest, unwrap } from '@/services/client';

describe('v1 API client', () => {
  it('unwraps the standard success envelope', () => {
    expect(unwrap({ code: 0, message: 'success', data: { id: 42 }, request_id: 'req-1' })).toEqual({ id: 42 });
  });

  it('keeps backend error code and request id', () => {
    expect(() => unwrap({ code: 40001, message: '资源不存在', data: null, request_id: 'req-2' }))
      .toThrowError(ApiError);
    try {
      unwrap({ code: 40001, message: '资源不存在', data: null, request_id: 'req-2' });
    } catch (error) {
      expect(error).toMatchObject({ code: '40001', requestId: 'req-2' });
    }
  });

  it('accepts an unwrapped response during migration', () => {
    expect(unwrap(['node-a'])).toEqual(['node-a']);
  });

  it('retries transient reads but never retries writes', () => {
    expect(shouldRetryRequest('GET', 502, undefined, true)).toBe(true);
    expect(shouldRetryRequest('get', undefined, 'ECONNABORTED', false)).toBe(true);
    expect(shouldRetryRequest('POST', 502, undefined, true)).toBe(false);
    expect(shouldRetryRequest('PUT', undefined, 'ERR_NETWORK', false)).toBe(false);
  });
});
