import type { ResourceSpec } from '@/pages/shared/types';

const commonAccountFields: ResourceSpec['fields'] = [
  { key: 'id', label: '账户标识', readonly: true, formHidden: true },
  { key: 'username', label: '账户名称', required: true, placeholder: '请输入唯一账户名称' },
  { key: 'balance', label: '账户余额', kind: 'number', readonly: true },
  { key: 'credit_limit', label: '授信额度', kind: 'number', min: 0, step: 0.001, defaultValue: 0 },
  {
    key: 'billing_interval_secs',
    label: '计费周期（秒）',
    kind: 'number',
    min: 1,
    required: true,
    defaultValue: 60,
  },
  {
    key: 'price_per_interval',
    label: '周期价格（元）',
    kind: 'number',
    min: 0,
    step: 0.001,
    required: true,
    defaultValue: 0,
  },
  { key: 'enabled', label: '启用状态', kind: 'switch', defaultValue: true },
  {
    key: 'trunk_ids',
    label: '关联中继',
    readonly: true,
    formHidden: true,
  },
  {
    key: 'created_at',
    label: '创建时间',
    readonly: true,
    kind: 'datetime',
    formHidden: true,
  },
];

export const accessAccounts: ResourceSpec = {
  title: '对接账户',
  description: '管理客户侧账户余额、计费规则与对接网关关联。充值直接进入选定账户。',
  path: '/billing/access-accounts',
  idKey: 'id',
  createLabel: '新建账户',
  action: 'credit',
  tableFieldLimit: 10,
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索账户名称或网关...' },
  ],
  fields: [
    ...commonAccountFields.slice(0, 2),
    {
      key: 'tenant_id',
      label: '所属租户',
      kind: 'select',
      optionsResource: 'tenants',
      required: false,
      tableHidden: true,
      placeholder: '选择开户租户（留空为全局）',
    },
    {
      key: 'gateway_ids',
      label: '对接中继',
      kind: 'multiselect',
      optionsResource: 'access-trunks',
      required: false,
      tableHidden: true,
      fullWidth: true,
      placeholder: '可选择多个对接中继，留空表示暂不关联',
    },
    ...commonAccountFields.slice(2),
  ],
};

export const egressAccounts: ResourceSpec = {
  title: '落地账户',
  description: '管理供应商侧账户余额、成本规则与落地网关关联，独立核算通话成本。',
  path: '/billing/egress-accounts',
  idKey: 'id',
  createLabel: '新建账户',
  action: 'credit',
  tableFieldLimit: 10,
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索账户名称或网关...' },
  ],
  fields: [
    ...commonAccountFields.slice(0, 2),
    {
      key: 'gateway_ids',
      label: '落地中继',
      kind: 'multiselect',
      optionsResource: 'egress-trunks',
      required: false,
      tableHidden: true,
      fullWidth: true,
      placeholder: '可选择多个落地中继，留空表示暂不关联',
    },
    ...commonAccountFields.slice(2),
  ],
};

export const billingCredits: ResourceSpec = {
  title: '充值记录',
  description: '审计每一笔账户充值，记录操作人员、充值时间和余额变化。',
  path: '/billing/credits',
  idKey: 'idempotency_key',
  readOnly: true,
  tableFieldLimit: 9,
  serverFilters: [
    { param: 'q', label: '搜索', kind: 'keyword', placeholder: '搜索账户或操作人员...' },
    {
      param: 'account_type',
      label: '账户类型',
      kind: 'select',
      options: [
        { label: '对接账户', value: 'access' },
        { label: '落地账户', value: 'egress' },
      ],
    },
    {
      param: 'time',
      label: '充值时间',
      kind: 'dateRange',
      startParam: 'start_time',
      endParam: 'end_time',
    },
  ],
  fields: [
    { key: 'created_at', label: '充值时间', kind: 'datetime' },
    {
      key: 'account_type',
      label: '账户类型',
      kind: 'select',
      options: [
        { label: '对接账户', value: 'access' },
        { label: '落地账户', value: 'egress' },
      ],
    },
    { key: 'username', label: '账户名称' },
    { key: 'amount', label: '充值金额（元）', kind: 'number' },
    { key: 'balance_before', label: '充值前余额（元）', kind: 'number' },
    { key: 'balance_after', label: '充值后余额（元）', kind: 'number' },
    { key: 'operator_username', label: '操作人员' },
    { key: 'remark', label: '充值说明' },
    { key: 'idempotency_key', label: '业务流水号' },
  ],
};
