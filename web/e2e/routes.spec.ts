import { expect, test, type Page, type Route } from '@playwright/test';

const SESSION_KEY = 'vos-auth-session';
const ADMIN_SESSION = { token: 'e2e-admin-token', username: 'admin', role: 'admin' };

interface RouteCase {
  path: string;
  visible: string | RegExp;
  fieldValue?: string;
}

const staticRoutes: RouteCase[] = [
  { path: '/overview', visible: '电信软交换运行总览' },
  { path: '/copilot', visible: /LLM Telecom Copilot/ },
  { path: '/calls/active', visible: '活跃通话监控' },
  { path: '/calls', visible: '通话记录' },
  { path: '/extensions', visible: '分机' },
  { path: '/numbers', visible: '号码库存' },
  { path: '/did-destinations', visible: '呼入目标' },
  { path: '/trunks/access', visible: '接入中继' },
  { path: '/trunks/egress', visible: '落地中继' },
  { path: '/caller-pools', visible: '号码池组' },
  { path: '/egress-groups', visible: '落地分组' },
  { path: '/queues', visible: '呼叫队列总数' },
  { path: '/agents', visible: '总座席数' },
  { path: '/ivr', visible: 'IVR 多级语音导航' },
  { path: '/routing', visible: '路由策略编排' },
  { path: '/billing/accounts', visible: '多租户计费账户与月度账单' },
  { path: '/billing/rates', visible: '费率' },
  { path: '/billing/transactions', visible: '账务流水' },
  { path: '/security', visible: '安全策略' },
  { path: '/infrastructure', visible: '软交换集群节点管理' },
  { path: '/settings', visible: '核心运行参数设置' },
];

const detailRoutes: RouteCase[] = [
  { path: '/calls/call-e2e', visible: 'call-e2e' },
  { path: '/extensions/1001', visible: '分机 1001' },
  { path: '/trunks/access/access-e2e', visible: '中继 access-e2e' },
  { path: '/trunks/egress/egress-e2e', visible: '中继 egress-e2e' },
  { path: '/trunks/egress-e2e', visible: '中继 egress-e2e' },
  { path: '/caller-pools/pool-e2e', visible: '基本配置', fieldValue: 'pool-e2e' },
  { path: '/egress-groups/group-e2e', visible: '落地分组', fieldValue: 'group-e2e' },
];

function pageResult(items: unknown[] = []) {
  return { items, pagination: { page: 1, page_size: 20, total: items.length, total_pages: 1 } };
}

function mockPayload(pathname: string): unknown {
  if (pathname === '/api/v1/overview/summary') return {};
  if (pathname === '/api/v1/calls/active') return [];
  if (pathname === '/api/v1/calls/call-e2e') {
    return { call_id: 'call-e2e', caller: '1001', callee: '13800138000', status: 'answered', duration_ms: 60_000 };
  }
  if (pathname === '/api/v1/extensions/1001') {
    return { extension: { username: '1001' }, registrations: [], numbers: [], credential: { configured: true } };
  }
  if (pathname === '/api/v1/extensions/1001/outbound-policy') return outboundPolicy();
  if (/^\/api\/v1\/trunks\/[^/]+\/ip-rules$/.test(pathname)) return [];
  if (/^\/api\/v1\/trunks\/[^/]+\/egress-endpoints$/.test(pathname)) return [];
  if (/^\/api\/v1\/trunks\/[^/]+\/outbound-policy$/.test(pathname)) return outboundPolicy();
  if (/^\/api\/v1\/trunks\/[^/]+$/.test(pathname)) {
    const id = decodeURIComponent(pathname.split('/').at(-1) ?? 'egress-e2e');
    const access = id.startsWith('access');
    return {
      trunk: {
        id,
        role: access ? 'access' : 'egress',
        access_auth_mode: access ? 'ip_allowlist' : undefined,
        host: access ? '' : '192.0.2.10',
        port: 5060,
        transport: 'udp',
        max_capacity: 100,
        enabled: true,
      },
      registrations: [],
      numbers: [],
    };
  }
  if (/^\/api\/v1\/caller-pools\/pool-e2e\/members$/.test(pathname)) return [];
  if (pathname === '/api/v1/caller-pools') {
    return [{ id: 'pool-e2e', virtual_alias: 'virtual-e2e', owner_source_type: 'trunk', owner_source_id: 'access-e2e', strategy: 'random', fallback_mode: 'reject', enabled: true }];
  }
  if (/^\/api\/v1\/egress-groups\/group-e2e\/members$/.test(pathname)) return [];
  if (pathname === '/api/v1/egress-groups') {
    return [{ id: 'group-e2e', name: 'E2E 落地分组', description: 'route smoke fixture', enabled: true }];
  }
  if (pathname === '/api/v1/call-center/queues' || pathname === '/api/v1/call-center/agents' || pathname === '/api/v1/ivr/menus') {
    return { items: [] };
  }
  if (pathname === '/api/v1/infrastructure/sip-cluster') return { nodes: [] };
  if (pathname === '/api/v1/infrastructure/settings') return { values: { configs: {} } };
  if (pathname === '/api/v1/copilot/chat') return { answer: 'E2E response' };
  return pageResult();
}

function outboundPolicy() {
  return {
    caller_mode: 'strict_passthrough',
    fallback_mode: 'reject',
    egress_mode: 'direct',
    direct_egress_trunk_id: 'egress-e2e',
    enabled: true,
  };
}

async function fulfillApi(route: Route) {
  const request = route.request();
  if (request.method() === 'OPTIONS') {
    await route.fulfill({ status: 204 });
    return;
  }
  const pathname = new URL(request.url()).pathname;
  await route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ code: 0, message: 'success', data: mockPayload(pathname), request_id: 'e2e-request' }),
  });
}

async function configureAuthenticatedPage(page: Page) {
  await page.addInitScript(([key, session]) => {
    window.localStorage.setItem(key, JSON.stringify(session));
  }, [SESSION_KEY, ADMIN_SESSION] as const);
  await page.route('**/api/v1/**', fulfillApi);
}

async function expectHealthyRoute(page: Page, routeCase: RouteCase) {
  const pageErrors: Error[] = [];
  page.on('pageerror', (error) => pageErrors.push(error));

  await page.goto(routeCase.path);
  await expect(page.getByText(routeCase.visible, { exact: typeof routeCase.visible === 'string' }).first()).toBeVisible();
  if (routeCase.fieldValue) {
    await expect(page.locator(`input[value="${routeCase.fieldValue}"]`).first()).toBeVisible();
  }
  await expect(page.getByText('应用发生意外错误')).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1)).toBe(true);
  expect(pageErrors, pageErrors.map((error) => error.message).join('\n')).toEqual([]);
}

test.describe('登录路由', () => {
  test('未登录时展示登录页且页面完整', async ({ page }) => {
    await page.goto('/login');
    await expect(page.getByRole('heading', { name: '欢迎登录控制台' })).toBeVisible();
    await expect(page.getByLabel('控制台账号')).toBeVisible();
    await expect(page.getByLabel('访问密码')).toBeVisible();
    await expect(page.getByText('应用发生意外错误')).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1)).toBe(true);
  });
});

test.describe('管理员静态路由', () => {
  for (const routeCase of staticRoutes) {
    test(`${routeCase.path} 可稳定渲染`, async ({ page }) => {
      await configureAuthenticatedPage(page);
      await expectHealthyRoute(page, routeCase);
    });
  }
});

test.describe('管理员详情路由', () => {
  for (const routeCase of detailRoutes) {
    test(`${routeCase.path} 可稳定渲染`, async ({ page }) => {
      await configureAuthenticatedPage(page);
      await expectHealthyRoute(page, routeCase);
    });
  }
});
