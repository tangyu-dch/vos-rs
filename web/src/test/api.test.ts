import { describe, it, expect, vi, beforeEach } from 'vitest';
import { ApiError, apiService, formatApiError } from '@/services/api';
import { isForbiddenError } from '@/services/auth';

// Mock axios
vi.mock('axios', () => ({
  default: {
    create: vi.fn(() => ({
      get: vi.fn(),
      post: vi.fn(),
      put: vi.fn(),
      delete: vi.fn(),
      interceptors: {
        request: { use: vi.fn() },
        response: { use: vi.fn() },
      },
    })),
  },
}));

describe('apiService', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('has getDashboardStats method', () => {
    expect(typeof apiService.getDashboardStats).toBe('function');
  });

  it('has getCdrs method', () => {
    expect(typeof apiService.getCdrs).toBe('function');
  });

  it('has getUsers method', () => {
    expect(typeof apiService.getUsers).toBe('function');
  });

  it('has getGateways method', () => {
    expect(typeof apiService.getGateways).toBe('function');
  });

  it('has getRoutes method', () => {
    expect(typeof apiService.getRoutes).toBe('function');
  });


  it('has getActiveCalls method', () => {
    expect(typeof apiService.getActiveCalls).toBe('function');
  });

  it('has terminateCall method', () => {
    expect(typeof apiService.terminateCall).toBe('function');
  });

  it('has SIP cluster status and node control methods', () => {
    expect(typeof apiService.getSipClusterStatus).toBe('function');
    expect(typeof apiService.controlSipClusterNode).toBe('function');
  });

  it('formats backend errors and keeps request id', () => {
    const error = formatApiError({
      response: {
        data: { error: '无权访问该接口' },
        headers: { 'x-request-id': 'req_test_001' },
      },
    });

    expect(error.message).toBe('无权访问该接口（请求 ID: req_test_001）');
    expect(error).toBeInstanceOf(ApiError);
  });

  it('preserves HTTP status for permission handling', () => {
    const error = formatApiError({
      response: { status: 403, data: { message: '禁止访问' } },
    });

    expect(isForbiddenError(error)).toBe(true);
  });

});
