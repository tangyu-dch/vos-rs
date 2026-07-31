//! 动态访问控制、控制台用户与菜单资源表结构。

pub(crate) const CREATE_ACCESS_ROLES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS access_roles (
    role_key TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    is_system BOOLEAN NOT NULL DEFAULT FALSE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_ACCESS_PERMISSIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS access_permissions (
    permission_key TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    group_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_ACCESS_ROLE_PERMISSIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS access_role_permissions (
    role_key TEXT NOT NULL REFERENCES access_roles(role_key) ON DELETE CASCADE,
    permission_key TEXT NOT NULL REFERENCES access_permissions(permission_key) ON DELETE CASCADE,
    PRIMARY KEY (role_key, permission_key)
)
"#;

pub(crate) const CREATE_CONSOLE_USERS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS console_users (
    username TEXT PRIMARY KEY,
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL,
    role_key TEXT NOT NULL REFERENCES access_roles(role_key),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    is_builtin BOOLEAN NOT NULL DEFAULT FALSE,
    auth_version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_CONSOLE_USERS_ROLE_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_console_users_role ON console_users (role_key, enabled)";

pub(crate) const CREATE_MENU_GROUPS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS access_menu_groups (
    group_key TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    icon_key TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_MENU_ITEMS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS access_menu_items (
    item_key TEXT PRIMARY KEY,
    group_key TEXT NOT NULL REFERENCES access_menu_groups(group_key) ON DELETE CASCADE,
    label TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    icon_key TEXT NOT NULL,
    permission_key TEXT NOT NULL REFERENCES access_permissions(permission_key),
    sort_order INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const SEED_ACCESS_ROLES_SQL: &str = r#"
INSERT INTO access_roles (role_key, name, description, is_system) VALUES
('admin', '系统管理员', '拥有平台全部权限', TRUE)
ON CONFLICT (role_key) DO UPDATE SET is_system = EXCLUDED.is_system
"#;

pub(crate) const NORMALIZE_SYSTEM_ROLE_SQL: &str = r#"
UPDATE access_roles SET is_system = (role_key = 'admin')
WHERE is_system <> (role_key = 'admin')
"#;

pub(crate) const SEED_ACCESS_PERMISSIONS_SQL: &str = r#"
INSERT INTO access_permissions (permission_key, name, group_name, description) VALUES
('*', '全部权限', '系统权限', '允许访问所有受保护资源'),
('session.read', '读取会话', '系统权限', '读取当前登录用户资料'),
('overview.view', '查看总览', '运行中心', '查看运行概览和趋势'),
('copilot.use', '使用助手', '运行中心', '使用智能助手及本人会话'),
('copilot.execute', '执行建议', '运行中心', '审批并执行助手生成的操作'),
('notifications.view', '查看消息', '消息中心', '查看站内消息'),
('notifications.read', '处理消息', '消息中心', '标记消息为已读'),
('notifications.scan', '扫描告警', '运行中心', '手动触发告警扫描'),
('announcements.view', '查看公告', '消息中心', '查看公告管理列表及本人公告'),
('announcements.create', '新建公告', '消息中心', '创建公告草稿'),
('announcements.update', '编辑公告', '消息中心', '编辑公告内容和投放设置'),
('announcements.delete', '删除公告', '消息中心', '删除公告及阅读回执'),
('announcements.publish', '发布公告', '消息中心', '发布公告或安排定时投放'),
('calls.view', '查看通话', '通话分析', '查看通话、录音和信令'),
('calls.export', '导出话单', '通话分析', '导出通话记录'),
('calls.terminate', '挂断通话', '通话分析', '强制挂断活跃通话'),
('calls.barge', '强插通话', '通话分析', '强插并接管活跃通话媒体'),
('calls.play', '播放语音', '通话分析', '向活跃通话播放语音'),
('calls.mute', '静音通话', '通话分析', '设置活跃通话静音状态'),
('calls.monitor', '监听通话', '通话分析', '监听活跃通话媒体'),
('calls.transfer', '转接通话', '通话分析', '将活跃通话转接到指定目标'),
('registrations.view', '查看注册', '号码分机', '查看终端注册状态'),
('extensions.view', '查看分机', '号码分机', '查看分机'),
('extensions.create', '新建分机', '号码分机', '新建分机及认证凭据'),
('extensions.update', '编辑分机', '号码分机', '编辑分机及认证凭据'),
('extensions.delete', '删除分机', '号码分机', '删除分机'),
('extensions.import', '导入分机', '号码分机', '批量导入分机'),
('extensions.export', '导出分机', '号码分机', '导出分机'),
('numbers.view', '查看号码', '号码分机', '查看号码和呼入目标'),
('numbers.create', '新建号码', '号码分机', '新建号码和呼入目标'),
('numbers.update', '编辑号码', '号码分机', '编辑号码和呼入目标'),
('numbers.delete', '删除号码', '号码分机', '删除号码和呼入目标'),
('numbers.import', '导入号码', '号码分机', '批量导入号码'),
('numbers.export', '导出号码', '号码分机', '导出号码'),
('trunks.view', '查看中继', '中继路由', '查看接入和落地中继'),
('trunks.create', '新建中继', '中继路由', '新建接入和落地中继'),
('trunks.update', '编辑中继', '中继路由', '编辑接入和落地中继'),
('trunks.delete', '删除中继', '中继路由', '删除接入和落地中继'),
('trunks.import', '导入中继', '中继路由', '批量导入中继'),
('trunks.export', '导出中继', '中继路由', '导出中继'),
('routing.view', '查看路由', '中继路由', '查看路由规则'),
('routing.create', '新建路由', '中继路由', '新建路由规则'),
('routing.update', '编辑路由', '中继路由', '编辑路由规则'),
('routing.delete', '删除路由', '中继路由', '删除路由规则'),
('routing.import', '导入路由', '中继路由', '批量导入路由规则'),
('routing.export', '导出路由', '中继路由', '导出路由规则'),
('routing.simulate', '路由试算', '中继路由', '执行路由试算'),
('termination.view', '查看落地', '中继路由', '查看号码池、落地组和外呼策略'),
('termination.manage', '管理落地', '中继路由', '修改号码池、落地组和外呼策略'),
('queues.view', '查看队列', '呼叫中心', '查看呼叫队列'),
('queues.create', '新建队列', '呼叫中心', '新建呼叫队列'),
('queues.update', '编辑队列', '呼叫中心', '编辑呼叫队列'),
('queues.delete', '删除队列', '呼叫中心', '删除呼叫队列'),
('queues.export', '导出队列', '呼叫中心', '导出呼叫队列'),
('agents.view', '查看坐席', '呼叫中心', '查看坐席'),
('agents.create', '新建坐席', '呼叫中心', '新建坐席'),
('agents.update', '编辑坐席', '呼叫中心', '编辑坐席'),
('agents.delete', '删除坐席', '呼叫中心', '删除坐席'),
('agents.export', '导出坐席', '呼叫中心', '导出坐席'),
('ivr.view', '查看导航', '呼叫中心', '查看语音导航'),
('ivr.create', '新建导航', '呼叫中心', '新建语音导航'),
('ivr.update', '编辑导航', '呼叫中心', '编辑语音导航'),
('ivr.delete', '删除导航', '呼叫中心', '删除语音导航'),
('ivr.prompts', '管理提示', '呼叫中心', '管理语音提示文件'),
('billing.access_accounts.view', '查看对接账户', '计费账务', '查看对接账户'),
('billing.access_accounts.create', '新建对接账户', '计费账务', '新建对接账户'),
('billing.access_accounts.update', '编辑对接账户', '计费账务', '编辑对接账户'),
('billing.access_accounts.delete', '删除对接账户', '计费账务', '删除对接账户'),
('billing.access_accounts.credit', '充值对接账户', '计费账务', '向对接账户充值'),
('billing.access_accounts.export', '导出对接账户', '计费账务', '导出对接账户'),
('billing.egress_accounts.view', '查看落地账户', '计费账务', '查看落地账户'),
('billing.egress_accounts.create', '新建落地账户', '计费账务', '新建落地账户'),
('billing.egress_accounts.update', '编辑落地账户', '计费账务', '编辑落地账户'),
('billing.egress_accounts.delete', '删除落地账户', '计费账务', '删除落地账户'),
('billing.egress_accounts.credit', '充值落地账户', '计费账务', '向落地账户充值'),
('billing.egress_accounts.export', '导出落地账户', '计费账务', '导出落地账户'),
('billing.credits.view', '查看充值流水', '计费账务', '查看充值及账务流水'),
('billing.credits.export', '导出充值流水', '计费账务', '导出充值及账务流水'),
('billing.ledger.view', '查看流水', '计费账务', '查看账务流水'),
('billing.ledger.export', '导出流水', '计费账务', '导出账务流水'),
('security.view', '查看安全', '系统安全', '查看反欺诈规则和安全事件'),
('security.manage', '管理安全', '系统安全', '管理反欺诈规则和安全事件'),
('security.audit', '查看审计', '系统安全', '查看接口审计日志'),
('infrastructure.view', '查看集群', '系统安全', '查看信令与媒体节点'),
('infrastructure.manage', '管理集群', '系统安全', '管理信令与媒体节点'),
('tenants.view', '查看租户', '系统安全', '查看租户及关联账户'),
('tenants.create', '新建租户', '系统安全', '新建租户'),
('tenants.update', '编辑租户', '系统安全', '编辑租户'),
('tenants.delete', '删除租户', '系统安全', '删除租户'),
('tenants.export', '导出租户', '系统安全', '导出租户'),
('tenants.manage', '管理租户', '系统安全', '管理租户及关联账户'),
('llm.view', '查看模型', '系统安全', '查看大模型配置'),
('llm.create', '新增模型', '系统安全', '新增大模型配置'),
('llm.update', '修改模型', '系统安全', '修改大模型配置'),
('llm.delete', '删除模型', '系统安全', '删除大模型配置'),
('llm.activate', '启用模型', '系统安全', '切换当前启用的大模型配置'),
('llm.manage', '管理模型', '系统安全', '兼容原有大模型管理授权'),
('settings.view', '查看设置', '系统安全', '查看系统运行参数'),
('settings.manage', '管理设置', '系统安全', '管理系统运行参数'),
('access.view', '查看权限', '权限管理', '查看用户、角色、权限和菜单'),
('access.accounts.view', '查看账户', '权限管理', '查看控制台账户'),
('access.accounts.create', '新增账户', '权限管理', '新增控制台账户'),
('access.accounts.update', '修改账户', '权限管理', '修改控制台账户资料、角色、状态和密码'),
('access.accounts.delete', '删除账户', '权限管理', '删除非内置控制台账户'),
('access.users', '管理用户', '权限管理', '兼容原有账户管理授权'),
('access.roles', '管理角色', '权限管理', '新建角色及配置按钮权限'),
('access.roles.view', '查看角色', '权限管理', '查看角色及其权限'),
('access.roles.create', '新建角色', '权限管理', '新建动态角色'),
('access.roles.update', '修改角色', '权限管理', '修改角色名称、说明和状态'),
('access.roles.delete', '删除角色', '权限管理', '删除未分配账户的非内置角色'),
('access.roles.permissions', '配置权限', '权限管理', '配置角色的菜单和按钮权限'),
('access.roles.assign', '分配人员', '权限管理', '调整控制台账户的所属角色'),
('access.menus', '管理菜单', '权限管理', '配置菜单名称、顺序和启用状态')
ON CONFLICT (permission_key) DO UPDATE SET
name = EXCLUDED.name, group_name = EXCLUDED.group_name, description = EXCLUDED.description
"#;

pub(crate) const SEED_ACCESS_ROLE_PERMISSIONS_SQL: &str = r#"
INSERT INTO access_role_permissions (role_key, permission_key)
SELECT seeds.role_key, seeds.permission_key FROM (VALUES
('admin', '*'),
('operator', 'session.read'), ('operator', 'overview.view'),
('operator', 'copilot.use'), ('operator', 'copilot.execute'),
('operator', 'notifications.view'), ('operator', 'notifications.read'),
('operator', 'notifications.scan'), ('operator', 'calls.view'),
('operator', 'announcements.view'),
('operator', 'calls.export'), ('operator', 'calls.terminate'),
('operator', 'calls.barge'), ('operator', 'calls.transfer'),
('operator', 'calls.play'), ('operator', 'calls.mute'),
('operator', 'calls.monitor'), ('operator', 'registrations.view'),
('operator', 'numbers.view'), ('operator', 'numbers.create'),
('operator', 'numbers.update'), ('operator', 'numbers.delete'),
('operator', 'numbers.import'), ('operator', 'numbers.export'),
('operator', 'trunks.view'), ('operator', 'trunks.create'),
('operator', 'trunks.update'), ('operator', 'trunks.delete'),
('operator', 'trunks.import'), ('operator', 'trunks.export'),
('operator', 'routing.view'), ('operator', 'routing.create'),
('operator', 'routing.update'), ('operator', 'routing.delete'),
('operator', 'routing.import'), ('operator', 'routing.export'),
('operator', 'routing.simulate'),
('operator', 'termination.manage'),
('operator', 'termination.view'),
('operator', 'queues.view'), ('operator', 'queues.create'),
('operator', 'queues.update'), ('operator', 'queues.delete'),
('operator', 'queues.export'),
('operator', 'agents.view'), ('operator', 'agents.create'),
('operator', 'agents.update'), ('operator', 'agents.delete'),
('operator', 'agents.export'),
('operator', 'ivr.view'), ('operator', 'ivr.create'),
('operator', 'ivr.update'), ('operator', 'ivr.delete'),
('operator', 'ivr.prompts'), ('operator', 'security.view'),
('operator', 'security.manage'),
('financier', 'session.read'), ('financier', 'overview.view'),
('financier', 'copilot.use'), ('financier', 'notifications.view'),
('financier', 'notifications.read'), ('financier', 'calls.view'),
('financier', 'announcements.view'),
('financier', 'calls.export'), ('financier', 'registrations.view'),
('financier', 'billing.access_accounts.view'), ('financier', 'billing.access_accounts.create'),
('financier', 'billing.access_accounts.update'), ('financier', 'billing.access_accounts.delete'),
('financier', 'billing.access_accounts.credit'), ('financier', 'billing.access_accounts.export'),
('financier', 'billing.egress_accounts.view'), ('financier', 'billing.egress_accounts.create'),
('financier', 'billing.egress_accounts.update'), ('financier', 'billing.egress_accounts.delete'),
('financier', 'billing.egress_accounts.credit'), ('financier', 'billing.egress_accounts.export'),
('financier', 'billing.credits.view'), ('financier', 'billing.credits.export'),
('financier', 'billing.ledger.view'), ('financier', 'billing.ledger.export')
) AS seeds(role_key, permission_key)
JOIN access_roles roles ON roles.role_key = seeds.role_key
ON CONFLICT (role_key, permission_key) DO NOTHING
"#;

pub(crate) const MIGRATE_LEGACY_ACCESS_ROLE_PERMISSIONS_SQL: &[&str] = &[
    r#"
WITH legacy_roles AS (
    DELETE FROM access_role_permissions WHERE permission_key = 'access.users' RETURNING role_key
)
INSERT INTO access_role_permissions (role_key, permission_key)
SELECT legacy_roles.role_key, permissions.permission_key
FROM legacy_roles CROSS JOIN (VALUES
    ('access.accounts.view'), ('access.accounts.create'),
    ('access.accounts.update'), ('access.accounts.delete')
) AS permissions(permission_key)
ON CONFLICT (role_key, permission_key) DO NOTHING
"#,
    r#"
WITH legacy_roles AS (
    DELETE FROM access_role_permissions WHERE permission_key = 'access.roles' RETURNING role_key
)
INSERT INTO access_role_permissions (role_key, permission_key)
SELECT legacy_roles.role_key, permissions.permission_key
FROM legacy_roles CROSS JOIN (VALUES
    ('access.roles.view'), ('access.roles.create'), ('access.roles.update'),
    ('access.roles.delete'), ('access.roles.permissions'), ('access.roles.assign')
) AS permissions(permission_key)
ON CONFLICT (role_key, permission_key) DO NOTHING
"#,
    r#"
WITH legacy_roles AS (
    DELETE FROM access_role_permissions WHERE permission_key = 'llm.manage' RETURNING role_key
)
INSERT INTO access_role_permissions (role_key, permission_key)
SELECT legacy_roles.role_key, permissions.permission_key
FROM legacy_roles CROSS JOIN (VALUES
    ('llm.view'), ('llm.create'), ('llm.update'), ('llm.delete'), ('llm.activate')
) AS permissions(permission_key)
ON CONFLICT (role_key, permission_key) DO NOTHING
"#,
    r#"
DELETE FROM access_role_permissions
WHERE permission_key IN (
    'billing.accounts.view', 'billing.accounts.export', 'billing.accounts.credit',
    'billing.reconcile'
)
"#,
    "DELETE FROM access_menu_items WHERE item_key = 'accounts'",
    r#"
DELETE FROM access_permissions
WHERE permission_key IN (
    'billing.accounts.view', 'billing.accounts.export', 'billing.accounts.credit',
    'billing.reconcile'
)
"#,
    r#"
DELETE FROM access_role_permissions
WHERE permission_key LIKE 'billing.rates.%'
"#,
    "DELETE FROM access_menu_items WHERE item_key = 'rates'",
    r#"
DELETE FROM access_permissions
WHERE permission_key LIKE 'billing.rates.%'
"#,
];

pub(crate) const SEED_MENU_GROUPS_SQL: &str = r#"
INSERT INTO access_menu_groups (group_key, label, icon_key, sort_order) VALUES
('operations', '运行监控', 'activity', 10),
('messages', '智能运维', 'bot', 20),
('number_pools', '号码路由', 'book', 30),
('trunks', '中继管理', 'server', 40),
('call_center', '呼叫中心', 'grid', 50),
('analytics', '通话分析', 'phone', 60),
('billing', '计费账务', 'book', 70),
('security', '系统管理', 'shield', 80)
ON CONFLICT (group_key) DO UPDATE SET
label = EXCLUDED.label, icon_key = EXCLUDED.icon_key, sort_order = EXCLUDED.sort_order
"#;

pub(crate) const SEED_MENU_ITEMS_SQL: &str = r#"
INSERT INTO access_menu_items
(item_key, group_key, label, path, icon_key, permission_key, sort_order) VALUES
('overview', 'operations', '运行总览', '/overview', 'dashboard', 'overview.view', 10),
('rwi', 'operations', '实时控制', '/rwi', 'radio', 'calls.monitor', 20),
('copilot', 'messages', '智能助手', '/copilot', 'bot', 'copilot.use', 10),
('active_calls', 'operations', '活跃通话', '/calls/active', 'phone', 'calls.view', 40),
('notifications', 'messages', '消息通知', '/notifications', 'bell', 'notifications.view', 20),
('announcements', 'messages', '公告管理', '/announcements', 'book', 'announcements.view', 30),
('numbers', 'number_pools', '号码库', '/numbers', 'book', 'numbers.view', 10),
('caller_pools', 'number_pools', '号码池组', '/caller-pools', 'grid', 'termination.view', 20),
('routes', 'number_pools', '呼出路由', '/routing', 'fork', 'routing.view', 30),
('did', 'number_pools', '呼入目标', '/did-destinations', 'branch', 'termination.view', 40),
('access_trunks', 'trunks', '接入中继', '/trunks/access', 'server', 'trunks.view', 10),
('egress_trunks', 'trunks', '落地中继', '/trunks/egress', 'server', 'trunks.view', 20),
('egress_groups', 'trunks', '落地分组', '/egress-groups', 'branch', 'termination.view', 30),
('extensions', 'call_center', '分机管理', '/extensions', 'users', 'extensions.view', 10),
('ivr', 'call_center', '语音导航', '/ivr', 'branch', 'ivr.view', 20),
('queues', 'call_center', '呼叫队列', '/queues', 'grid', 'queues.view', 30),
('agents', 'call_center', '座席监控', '/agents', 'users', 'agents.view', 40),
('calls', 'analytics', '通话记录', '/calls', 'phone', 'calls.view', 10),
('access_billing_accounts', 'billing', '对接账户', '/billing/access-accounts', 'users', 'billing.access_accounts.view', 10),
('egress_billing_accounts', 'billing', '落地账户', '/billing/egress-accounts', 'users', 'billing.egress_accounts.view', 20),
('billing_credits', 'billing', '充值记录', '/billing/credits', 'book', 'billing.credits.view', 30),
('transactions', 'billing', '账务流水', '/billing/transactions', 'book', 'billing.ledger.view', 50),
('security', 'security', '安全策略', '/security', 'shield', 'security.view', 10),
('infrastructure', 'security', '集群节点', '/infrastructure', 'alert', 'infrastructure.view', 20),
('tenants', 'security', '租户管理', '/tenants', 'building', 'tenants.view', 30),
('llm', 'security', '模型配置', '/settings/llm', 'cpu', 'llm.view', 40),
('access_control', 'security', '用户管理', '/access-control/accounts', 'users', 'access.accounts.view', 50),
('role_permissions', 'security', '角色权限', '/access-control/roles', 'key', 'access.roles.view', 60),
('settings', 'security', '系统设置', '/settings', 'settings', 'settings.view', 70)
ON CONFLICT (item_key) DO UPDATE SET
group_key = EXCLUDED.group_key, label = EXCLUDED.label, path = EXCLUDED.path,
icon_key = EXCLUDED.icon_key, permission_key = EXCLUDED.permission_key,
sort_order = EXCLUDED.sort_order
"#;

pub(crate) const REMOVE_LEGACY_BILLING_ACCOUNT_MENU_SQL: &str =
    "DELETE FROM access_menu_items WHERE item_key = 'accounts'";
