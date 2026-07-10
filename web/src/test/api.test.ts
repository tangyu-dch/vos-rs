import { describe, it, expect, vi, beforeEach } from 'vitest';
import { apiService } from '@/services/api';

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

  it('has getRecordings method', () => {
    expect(typeof apiService.getRecordings).toBe('function');
  });

  it('has getActiveCalls method', () => {
    expect(typeof apiService.getActiveCalls).toBe('function');
  });

  it('has terminateCall method', () => {
    expect(typeof apiService.terminateCall).toBe('function');
  });

});
