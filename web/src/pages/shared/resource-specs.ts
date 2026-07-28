// 资源规格定义：分机/中继/号码/计费/通话等
// 从 console.tsx 拆分

import type { ResourceSpec } from '@/pages/shared/types';

export const extensions: ResourceSpec = {
  title: '分机', description: '管理 SIP 账号凭据与呼叫身份。', path: '/extensions',
  idKey: 'username', detailPath: '/extensions', createLabel: '新建分机',
  fields: [
    { key: 'username', label: '分机号', required: true, placeholder: '例如 1001' },
    { key: 'tenant_id', label: '所属商户', kind: 'select', optionsResource: 'tenants', placeholder: '选择所属商户（留空为全局）' },
    { key: 'registration_status', label: '注册状态', readonly: true },
    { key: 'sip_domain', label: '注册服务器', readonly: true },
    { key: 'realm', label: '鉴权域 (Realm)', readonly: true },
    { key: 'password', label: 'SIP 密码', kind: 'secret', required: true, preserveEmptyOnEdit: true, placeholder: '编辑时保留空表示不修改密码' },
    { key: 'created_at', label: '创建时间', readonly: true, kind: 'datetime' },
  ],
};

export const accessTrunks: ResourceSpec = {
  title: '接入中继', description: '配置客户接入认证、安全防范与主叫匹配规则。', path: '/trunks',
  params: { role: 'access' }, idKey: 'id', detailPath: '/trunks/access', createLabel: '新建接入中继',
  fields: [
    { key: 'id', label: '中继标识', required: true, placeholder: '例如 customer-a' },
    { key: 'access_auth_mode', label: '认证方式', kind: 'select', required: true, options: [{ label: 'IP 白名单', value: 'ip_allowlist' }, { label: '注册认证', value: 'digest_register' }, { label: 'IP 加认证', value: 'ip_and_digest' }], defaultValue: 'ip_allowlist' },
    { key: 'access_username', label: '注册用户', required: true, showWhen: (draft) => ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode)) },
    { key: 'access_realm', label: '认证 Realm', required: true, defaultValue: 'vos-rs', showWhen: (draft) => ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode)) },
    { key: 'access_password', label: '注册密码', kind: 'secret', required: true, preserveEmptyOnEdit: true, showWhen: (draft) => ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode)) },
    { key: 'max_capacity', label: '容量上限', kind: 'number', defaultValue: 100 },
    { key: 'account_id', label: '计费账户', kind: 'select', optionsResource: 'accounts', placeholder: '选择主叫扣费账户' },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
    { key: 'host', label: '内部主机', readonly: true, defaultValue: '' },
    { key: 'port', label: '内部端口', readonly: true, defaultValue: 5060 },
    { key: 'transport', label: '内部协议', readonly: true, defaultValue: 'udp' },
  ],
};

export const egressTrunks: ResourceSpec = {
  title: '落地中继', description: '管理对接上游运营商网关端点、计费账户与容量上限。', path: '/trunks',
  params: { role: 'egress' }, idKey: 'id', detailPath: '/trunks/egress', createLabel: '新建落地中继',
  fields: [
    { key: 'id', label: '中继标识', required: true, placeholder: '例如 carrier-a' },
    { key: 'host', label: '对端主机地址', required: true, placeholder: '对端 IP 地址' },
    { key: 'port', label: 'SIP 端口', kind: 'number', defaultValue: 5060, required: true },
    { key: 'transport', label: '传输协议', kind: 'select', required: true, options: [{ label: 'UDP', value: 'udp' }, { label: 'TCP', value: 'tcp' }, { label: 'TLS', value: 'tls' }], defaultValue: 'udp' },
    { key: 'max_capacity', label: '容量上限', kind: 'number', defaultValue: 100 },
    { key: 'account_id', label: '计费账户', kind: 'select', optionsResource: 'accounts', placeholder: '选择落地成本/扣费账户' },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const numbers: ResourceSpec = {
  title: '号码库存', description: '管理真实号码的唯一落地归属、使用方向和分机授权。', path: '/numbers',
  idKey: 'number', createLabel: '录入号码',
  fields: [
    { key: 'number', label: '真实号码', required: true },
    { key: 'owner_egress_trunk_id', label: '落地中继', kind: 'select', optionsResource: 'egress-trunks', required: true, placeholder: '选择号码的唯一物理归属' },
    { key: 'allocation_source_type', label: '授权类型', kind: 'select', required: true, options: [{ label: '接入中继', value: 'trunk' }, { label: '分机号码', value: 'extension' }], defaultValue: 'extension' },
    { key: 'allocation_source_id', label: '授权对象', kind: 'select', optionsResource: 'allocation-source', required: true, placeholder: '选择已存在的接入中继或分机' },
    { key: 'max_concurrent', label: '号码并发', kind: 'number', defaultValue: 1 },
    { key: 'can_receive', label: '允许呼入', kind: 'switch', defaultValue: true },
    { key: 'can_present', label: '允许显号', kind: 'switch', defaultValue: true },
    { key: 'status', label: '号码状态', kind: 'select', required: true, options: [{ label: '可用号码', value: 'available' }, { label: '已分配', value: 'assigned' }, { label: '停用号码', value: 'disabled' }], defaultValue: 'available' },
  ],
};

export const accounts: ResourceSpec = {
  title: '计费账户', description: '查看余额、授信额度、币种、账户可用状态及关联租户。', path: '/billing/accounts',
  idKey: 'username', readOnly: true, action: 'credit',
  fields: [
    { key: 'username', label: '账户' },
    { key: 'balance', label: '余额', kind: 'number' },
    { key: 'credit_limit', label: '授信额度', kind: 'number' },
    { key: 'currency', label: '币种' },
    { key: 'associated_tenants_summary', label: '关联租户', readonly: true },
    { key: 'created_at', label: '创建时间', readonly: true, kind: 'datetime' },
  ],
};

export const rates: ResourceSpec = {
  title: '费率', description: '按号码前缀配置计费周期与周期价格。', path: '/billing/rates',
  idKey: 'id', createLabel: '新建费率',
  fields: [
    { key: 'id', label: '费率 ID', required: true },
    { key: 'prefix', label: '号码前缀', placeholder: '留空表示默认费率' },
    { key: 'tenant_id', label: '所属商户', kind: 'select', optionsResource: 'tenants', placeholder: '留空为全局费率' },
    { key: 'billing_interval_secs', label: '计费周期（秒）', kind: 'number', min: 1, required: true, defaultValue: 60 },
    { key: 'price_per_interval', label: '周期价格（元）', kind: 'number', min: 0.001, required: true, defaultValue: 0.5 },
    { key: 'rate_per_minute', label: '等效每分钟费率（元）', readonly: true, kind: 'number' },
    { key: 'description', label: '说明', kind: 'textarea', fullWidth: true },
    { key: 'created_at', label: '创建时间', readonly: true, kind: 'datetime' },
  ],
};

export const transactions: ResourceSpec = {
  title: '账务流水', description: '按通话追踪扣费、余额变化和处理结果。', path: '/billing/transactions',
  idKey: 'id', readOnly: true,
  fields: [
    { key: 'id', label: '流水号' },
    { key: 'call_id', label: '通话 ID' },
    { key: 'username', label: '账户' },
    { key: 'duration_ms', label: '计费时长（秒）', kind: 'duration' },
    { key: 'billing_interval_secs', label: '计费周期（秒）' },
    { key: 'price_per_interval', label: '周期价格（元）' },
    { key: 'amount', label: '金额（元）' },
    { key: 'balance_after', label: '余额（元）' },
    { key: 'created_at', label: '发生时间', kind: 'datetime' },
  ],
};

export const calls: ResourceSpec = {
  title: '通话记录', description: '查询呼叫结果、时长、路由和媒体质量。', path: '/calls',
  idKey: 'call_id', detailPath: '/calls', readOnly: true,
  fields: [
    { key: 'call_id', label: '通话 ID' },
    { key: 'caller', label: '主叫' },
    { key: 'callee', label: '被叫' },
    { key: 'direction', label: '方向' },
    { key: 'status', label: '结果' },
    { key: 'duration_ms', label: '时长（秒）', kind: 'duration' },
    { key: 'started_at_ms', label: '开始时间' },
  ],
};

export const didDestinations: ResourceSpec = {
  title: '呼入目标', description: '真实 DID 通过归属落地中继校验后转入指定业务目标。', path: '/did-destinations',
  idKey: 'number', createLabel: '新建目标',
  fields: [
    { key: 'number', label: 'DID 号码', required: true },
    { key: 'tenant_id', label: '租户标识', placeholder: '可选' },
    { key: 'target_type', label: '目标类型', kind: 'select', required: true, options: [{ label: '分机号码', value: 'extension' }, { label: '分机组', value: 'extension_group' }, { label: '本地 IVR', value: 'ivr' }, { label: '拒绝呼叫', value: 'reject' }], defaultValue: 'extension' },
    { key: 'target_id', label: '目标标识', required: true, placeholder: '分机号 / 分机组 ID / IVR 菜单 ID', showWhen: (draft) => draft.target_type !== 'reject' },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const callerPools: ResourceSpec = {
  title: '号码池组', description: '维护虚拟主叫别名、选号算法和真实号码成员。', path: '/caller-pools',
  idKey: 'id', detailPath: '/caller-pools', createLabel: '新建号码池',
  fields: [
    { key: 'id', label: '号码池 ID', required: true },
    { key: 'virtual_alias', label: '虚拟主叫', required: true },
    { key: 'owner_source_type', label: '来源类型', kind: 'select', required: true, options: [{ label: '接入中继', value: 'trunk' }, { label: '分机号码', value: 'extension' }], defaultValue: 'trunk' },
    { key: 'owner_source_id', label: '来源标识', kind: 'select', optionsResource: 'allocation-source', required: true, placeholder: '选择已存在的接入中继或分机' },
    { key: 'strategy', label: '选号算法', kind: 'select', required: true, options: [{ label: '均匀随机', value: 'random' }, { label: '权重随机', value: 'weighted_random' }, { label: '顺序轮询', value: 'round_robin' }, { label: '稳定哈希', value: 'stable_hash' }], defaultValue: 'random' },
    { key: 'fallback_mode', label: '失败处理', kind: 'select', required: true, options: [{ label: '拒绝呼叫', value: 'reject' }], defaultValue: 'reject' },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const egressGroups: ResourceSpec = {
  title: '落地分组', description: '定义来源允许使用的落地范围、目的地能力和故障边界。', path: '/egress-groups',
  idKey: 'id', detailPath: '/egress-groups', createLabel: '新建分组',
  fields: [
    { key: 'id', label: '分组 ID', required: true },
    { key: 'name', label: '分组名称', required: true },
    { key: 'description', label: '分组说明', kind: 'textarea', fullWidth: true },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const ivrMenus: ResourceSpec = {
  title: 'IVR 语音导航',
  description: '支持多级多节点拖拽编排，18 种节点类型覆盖播放/收号/分支/转接/AI 对话等场景。',
  path: '/ivr/menus',
  idKey: 'id',
  createLabel: '定义新 IVR',
  fields: [
    { key: 'id', label: 'IVR ID', required: true, placeholder: '例如 ivr-main-sales' },
    { key: 'name', label: '流程名称', required: true, placeholder: '例如 售前客服多级导航' },
    { key: 'did', label: '绑定 DID 号码', placeholder: '例如 4008009000' },
    { key: 'welcome_prompt', label: '欢迎语音文件', required: true, defaultValue: 'welcome.wav' },
    { key: 'node_count', label: '节点数', readonly: true },
    { key: 'timeout_secs', label: '全局超时 (秒)', kind: 'number', required: true, defaultValue: 30, min: 1 },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
    { key: 'description', label: '流程描述', kind: 'textarea', fullWidth: true, placeholder: '简要描述此 IVR 流程的用途' },
  ],
};

export const tenants: ResourceSpec = {
  title: '租户管理',
  description: '维护多租户隔离策略、并发/CPS 上限、网关白名单与统一计费账户关联。',
  path: '/tenants',
  idKey: 'id',
  createLabel: '新建租户',
  fields: [
    { key: 'name', label: '租户名称', required: true, placeholder: '例如 客户A-售前中心' },
    { key: 'domain', label: '租户域', required: true, placeholder: '例如 tenant-a.example.com' },
    { key: 'billing_account_summary', label: '关联计费账户', readonly: true },
    { key: 'max_concurrent_calls', label: '最大并发', kind: 'number', defaultValue: 100, min: 0 },
    { key: 'max_cps', label: '最大 CPS', kind: 'number', defaultValue: 10, min: 0 },
    {
      key: 'cross_tenant_policy',
      label: '跨租户策略',
      kind: 'select',
      required: true,
      defaultValue: 'allow_if_same_domain',
      options: [
        { label: '同域允许', value: 'allow_if_same_domain' },
        { label: '全部允许', value: 'allow' },
        { label: '全部拒绝', value: 'deny' },
      ],
    },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
    // 以下字段仅在编辑表单中显示，不作为表格列（visibleFields 截断后不显示）
    { key: 'id', label: '租户 ID', readonly: true, placeholder: '创建后由系统生成 UUID' },
    { key: 'recording_enabled', label: '启用录音', kind: 'switch', defaultValue: true },
    { key: 'billing_account_id', label: '计费账户', kind: 'select', optionsResource: 'accounts', placeholder: '选择租户统一计费账户' },
  ],
};
