// 资源规格定义：分机/中继/号码/计费/通话等
// 从 console.tsx 拆分

import type { ResourceSpec, ServerFilterSpec } from '@/pages/shared/types';

/** 通用"启用状态"下拉选项（值传给后端 boolean 参数） */
const ENABLED_FILTER_OPTIONS: ServerFilterSpec['options'] = [
  { label: '全部', value: '' },
  { label: '已启用', value: 'true' },
  { label: '已停用', value: 'false' },
];

export const extensions: ResourceSpec = {
  title: '分机管理',
  description: '管理分机账号凭据与呼叫身份。',
  path: '/extensions',
  idKey: 'username',
  detailPath: '/extensions',
  createLabel: '新建分机',
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索分机号...' },
    { param: 'tenant_id', label: '所属商户', kind: 'select', optionsResource: 'tenants' },
  ],
  fields: [
    { key: 'username', label: '分机号', required: true, placeholder: '例如 1001' },
    {
      key: 'tenant_id',
      label: '所属商户',
      kind: 'select',
      optionsResource: 'tenants',
      placeholder: '选择所属商户（留空为全局）',
    },
    { key: 'registration_status', label: '注册状态', readonly: true },
    { key: 'sip_domain', label: '注册服务器', readonly: true },
    { key: 'realm', label: '鉴权域', readonly: true },
    {
      key: 'password',
      label: '注册密码',
      kind: 'secret',
      required: true,
      preserveEmptyOnEdit: true,
      placeholder: '编辑时留空表示不修改密码',
    },
    { key: 'created_at', label: '创建时间', readonly: true, kind: 'datetime' },
  ],
};

export const accessTrunks: ResourceSpec = {
  title: '接入中继',
  description: '配置客户接入认证、安全防范与主叫匹配规则。',
  path: '/trunks',
  params: { role: 'access' },
  idKey: 'id',
  detailPath: '/trunks/access',
  createLabel: '新建接入中继',
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索中继标识或主机地址' },
    { param: 'enabled', label: '启用状态', kind: 'status', options: ENABLED_FILTER_OPTIONS },
  ],
  fields: [
    { key: 'id', label: '中继标识', required: true, placeholder: '例如 customer-a' },
    {
      key: 'tenant_id',
      label: '所属租户',
      kind: 'select',
      optionsResource: 'tenants',
      required: true,
      placeholder: '必须选择开户租户',
    },
    {
      key: 'access_auth_mode',
      label: '认证方式',
      kind: 'select',
      required: true,
      options: [
        { label: '地址白名单', value: 'ip_allowlist' },
        { label: '注册认证', value: 'digest_register' },
        { label: '组合认证', value: 'ip_and_digest' },
      ],
      defaultValue: 'ip_allowlist',
    },
    {
      key: 'host',
      label: 'IP 白名单 / 允许来源地址',
      required: true,
      fullWidth: true,
      placeholder: '请输入允许接入的 IP 地址或网关，支持多个 IP（逗号或分号分隔）',
      showWhen: (draft) =>
        ['ip_allowlist', 'ip_and_digest'].includes(String(draft.access_auth_mode)),
    },
    {
      key: 'access_username',
      label: '注册用户',
      required: true,
      showWhen: (draft) =>
        ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode)),
    },
    {
      key: 'access_realm',
      label: '认证域',
      required: true,
      defaultValue: 'vos-rs',
      showWhen: (draft) =>
        ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode)),
    },
    {
      key: 'access_password',
      label: '注册密码',
      kind: 'secret',
      required: true,
      preserveEmptyOnEdit: true,
      showWhen: (draft) =>
        ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode)),
    },
    { key: 'max_capacity', label: '容量上限', kind: 'number', defaultValue: 100 },
    {
      key: 'account_id',
      label: '对接账户',
      kind: 'select',
      optionsResource: 'access-accounts',
      placeholder: '选择对接扣费账户',
    },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
    { key: 'port', label: '内部端口', readonly: true, defaultValue: 5060, formHidden: true },
    { key: 'transport', label: '内部协议', readonly: true, defaultValue: 'udp', formHidden: true },
  ],
};

export const egressTrunks: ResourceSpec = {
  title: '落地中继',
  description: '管理对接上游运营商网关端点、计费账户与容量上限。',
  path: '/trunks',
  params: { role: 'egress' },
  idKey: 'id',
  detailPath: '/trunks/egress',
  createLabel: '新建落地中继',
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索中继标识或主机地址' },
    { param: 'enabled', label: '启用状态', kind: 'status', options: ENABLED_FILTER_OPTIONS },
  ],
  fields: [
    { key: 'id', label: '中继标识', required: true, placeholder: '例如 carrier-a' },
    { key: 'host', label: '对端主机地址', required: true, placeholder: '对端网络地址' },
    { key: 'port', label: '信令端口', kind: 'number', defaultValue: 5060, required: true },
    {
      key: 'transport',
      label: '传输协议',
      kind: 'select',
      required: true,
      options: [
        { label: '数据报传输', value: 'udp' },
        { label: '可靠传输', value: 'tcp' },
        { label: '加密传输', value: 'tls' },
      ],
      defaultValue: 'udp',
    },
    { key: 'max_capacity', label: '容量上限', kind: 'number', defaultValue: 100 },
    {
      key: 'account_id',
      label: '落地账户',
      kind: 'select',
      optionsResource: 'egress-accounts',
      placeholder: '选择落地成本/扣费账户',
    },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const numbers: ResourceSpec = {
  title: '号码管理',
  description: '管理真实号码的唯一落地归属、使用方向和分机授权。',
  path: '/numbers',
  idKey: 'number',
  createLabel: '录入号码',
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索号码...' },
    {
      param: 'status',
      label: '号码状态',
      kind: 'status',
      options: [
        { label: '全部', value: '' },
        { label: '可用号码', value: 'available' },
        { label: '已分配', value: 'assigned' },
        { label: '停用号码', value: 'disabled' },
      ],
    },
  ],
  fields: [
    { key: 'number', label: '真实号码', required: true },
    {
      key: 'tenant_id',
      label: '所属租户',
      kind: 'select',
      optionsResource: 'tenants',
      placeholder: '选择号码归属的开户租户 (留空表示系统公用)',
    },
    { key: 'max_concurrent', label: '号码并发', kind: 'number', defaultValue: 1 },
    { key: 'can_receive', label: '允许呼入', kind: 'switch', defaultValue: true },
    { key: 'can_present', label: '允许显号', kind: 'switch', defaultValue: true },
    {
      key: 'status',
      label: '号码状态',
      kind: 'select',
      required: true,
      options: [
        { label: '可用号码', value: 'available' },
        { label: '已分配', value: 'assigned' },
        { label: '停用号码', value: 'disabled' },
      ],
      defaultValue: 'available',
    },
  ],
};

export const didDestinations: ResourceSpec = {
  title: '呼入目标',
  description: '真实 DID 通过归属落地中继校验后转入指定业务目标。',
  path: '/did-destinations',
  idKey: 'number',
  createLabel: '新建目标',
  // 后端无分页/筛选，使用客户端筛选（数据量通常较小）
  serverFilters: [
    {
      param: 'q',
      label: '搜索',
      kind: 'keyword',
      placeholder: '搜索 DID / 目标...',
      mode: 'client',
      clientFields: ['number', 'target_id', 'tenant_id'],
    },
    {
      param: 'enabled',
      label: '启用状态',
      kind: 'status',
      options: ENABLED_FILTER_OPTIONS,
      mode: 'client',
    },
  ],
  fields: [
    { key: 'number', label: 'DID 号码', required: true },
    { key: 'tenant_id', label: '租户标识', placeholder: '可选' },
    {
      key: 'target_type',
      label: '目标类型',
      kind: 'select',
      required: true,
      options: [
        { label: '分机号码', value: 'extension' },
        { label: '分机组', value: 'extension_group' },
        { label: '语音导航', value: 'ivr' },
        { label: '拒绝呼叫', value: 'reject' },
      ],
      defaultValue: 'extension',
    },
    {
      key: 'target_id',
      label: '目标标识',
      required: true,
      placeholder: '填写分机、分机组或导航标识',
      showWhen: (draft) => draft.target_type !== 'reject',
    },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const callerPools: ResourceSpec = {
  title: '号码池组',
  description: '维护虚拟主叫别名、选号算法和真实号码成员。',
  path: '/caller-pools',
  idKey: 'id',
  createLabel: '新建号码池',
  customRowAction: {
    label: '配置成员',
    icon: 'Network',
    color: 'primary',
    onPress: (row, navigate) => {
      navigate(`/caller-pools/${encodeURIComponent(String(row.id))}`);
    },
  },
  // 后端无分页/筛选，使用客户端筛选
  serverFilters: [
    {
      param: 'q',
      label: '搜索',
      kind: 'keyword',
      placeholder: '搜索号码池 ID / 虚拟主叫...',
      mode: 'client',
      clientFields: ['id', 'virtual_alias'],
    },
    {
      param: 'enabled',
      label: '启用状态',
      kind: 'status',
      options: ENABLED_FILTER_OPTIONS,
      mode: 'client',
    },
  ],
  fields: [
    { key: 'id', label: '号码池 ID', formHidden: true, readonly: true },
    { key: 'virtual_alias', label: '虚拟主叫', required: true },
    {
      key: 'tenant_id',
      label: '所属租户',
      kind: 'select',
      optionsResource: 'tenants',
      placeholder: '选择号码池归属的开户租户',
    },
    {
      key: 'strategy',
      label: '选号算法',
      kind: 'select',
      required: true,
      options: [
        { label: '均匀随机', value: 'random' },
        { label: '权重随机', value: 'weighted_random' },
        { label: '顺序轮询', value: 'round_robin' },
        { label: '稳定哈希', value: 'stable_hash' },
      ],
      defaultValue: 'random',
    },
    {
      key: 'fallback_mode',
      label: '失败处理',
      kind: 'select',
      required: true,
      options: [{ label: '拒绝呼叫', value: 'reject' }],
      defaultValue: 'reject',
    },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const egressGroups: ResourceSpec = {
  title: '落地分组',
  description: '定义来源允许使用的落地范围、目的地能力和故障边界。',
  path: '/egress-groups',
  idKey: 'id',
  createLabel: '新建分组',
  customRowAction: {
    label: '配置成员',
    icon: 'Network',
    color: 'primary',
    onPress: (row, navigate) => {
      navigate(`/egress-groups/${encodeURIComponent(String(row.id))}`);
    },
  },
  // 后端无分页/筛选，使用客户端筛选
  serverFilters: [
    {
      param: 'q',
      label: '搜索',
      kind: 'keyword',
      placeholder: '搜索分组 ID / 名称...',
      mode: 'client',
      clientFields: ['id', 'name', 'description'],
    },
    {
      param: 'enabled',
      label: '启用状态',
      kind: 'status',
      options: ENABLED_FILTER_OPTIONS,
      mode: 'client',
    },
  ],
  fields: [
    { key: 'id', label: '分组 ID', formHidden: true, readonly: true },
    { key: 'name', label: '分组名称', required: true, placeholder: '请输入分组名称' },
    { key: 'description', label: '分组说明', kind: 'textarea', fullWidth: true },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  ],
};

export const ivrMenus: ResourceSpec = {
  title: '语音导航',
  description: '支持多级多节点拖拽编排，18 种节点类型覆盖播放/收号/分支/转接/AI 对话等场景。',
  path: '/ivr/menus',
  idKey: 'id',
  createLabel: '新建语音导航',
  // 后端响应虽声明分页但实际返回全部，使用客户端筛选
  serverFilters: [
    {
      param: 'q',
      label: '搜索',
      kind: 'keyword',
      placeholder: '搜索导航标识、名称或接入号码',
      mode: 'client',
      clientFields: ['id', 'name', 'did'],
    },
    {
      param: 'enabled',
      label: '启用状态',
      kind: 'status',
      options: ENABLED_FILTER_OPTIONS,
      mode: 'client',
    },
  ],
  fields: [
    { key: 'id', label: '导航标识', required: true, placeholder: '例如 main-sales' },
    { key: 'name', label: '流程名称', required: true, placeholder: '例如 售前客服多级导航' },
    { key: 'did', label: '绑定 DID 号码', placeholder: '例如 4008009000' },
    { key: 'welcome_prompt', label: '欢迎语音文件', required: true, defaultValue: 'welcome.wav' },
    { key: 'node_count', label: '节点数', readonly: true },
    {
      key: 'timeout_secs',
      label: '全局超时 (秒)',
      kind: 'number',
      required: true,
      defaultValue: 30,
      min: 1,
    },
    { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
    {
      key: 'description',
      label: '流程描述',
      kind: 'textarea',
      fullWidth: true,
      placeholder: '简要描述此导航流程的用途',
    },
  ],
};

export const tenants: ResourceSpec = {
  title: '租户管理',
  description: '维护多租户隔离策略、并发和每秒呼叫上限、网关白名单与关联对接计费账户。',
  path: '/tenants',
  idKey: 'id',
  createLabel: '新建租户',
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索租户名称 / 域...' },
    { param: 'enabled', label: '启用状态', kind: 'status', options: ENABLED_FILTER_OPTIONS },
  ],
  fields: [
    { key: 'name', label: '租户名称', required: true, placeholder: '例如 客户A-售前中心' },
    { key: 'domain', label: '租户域', required: true, placeholder: '例如 tenant-a.example.com' },
    { key: 'billing_summary', label: '计费状态', readonly: true, formHidden: true },
    { key: 'max_concurrent_calls', label: '最大并发', kind: 'number', defaultValue: 100, min: 0 },
    { key: 'max_cps', label: '每秒呼叫上限', kind: 'number', defaultValue: 10, min: 0 },
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
  ],
};

export const sipRoutes: ResourceSpec = {
  title: '呼出路由',
  description: '按开户租户与被叫前缀精准寻路，配置号段改写与落地中继。',
  path: '/routing/rules',
  idKey: 'id',
  createLabel: '新建路由规则',
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索前缀或目标中继...' },
    { param: 'tenant_id', label: '所属租户', kind: 'select', optionsResource: 'tenants' },
  ],
  fields: [
    { key: 'prefix', label: '匹配前缀', required: true, placeholder: '例如 86 或 010 (留空为全局默认)' },
    {
      key: 'tenant_id',
      label: '所属租户',
      kind: 'select',
      optionsResource: 'tenants',
      placeholder: '选择开户租户 (留空为公共路由)',
    },
    {
      key: 'gateway_id',
      label: '目标落地中继',
      kind: 'select',
      optionsResource: 'egress-trunks',
      required: true,
      placeholder: '选择落地中继线路',
    },
    { key: 'strip_prefix', label: '剪切前缀', placeholder: '呼出前剥离的前缀，如 0' },
    { key: 'add_prefix', label: '加头前缀', placeholder: '呼出前前置添加的前缀，如 86' },
    { key: 'priority', label: '优先级', kind: 'number', defaultValue: 100, required: true },
    { key: 'weight', label: '权重', kind: 'number', defaultValue: 100, required: true },
    { key: 'cost', label: '路由成本', kind: 'number', defaultValue: 0 },
  ],
};
