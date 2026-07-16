import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Alert, Button, Descriptions, Drawer, Empty, Form, Grid, Input, InputNumber, Message,
  Modal, Pagination, Popconfirm, Select, Space, Spin, Switch, Table, Tabs, Tag,
} from '@arco-design/web-react';
import { IconDelete, IconEdit, IconEye, IconPlus, IconRefresh, IconSearch, IconSend } from '@arco-design/web-react/icon';
import { useNavigate, useParams } from 'react-router-dom';
import { api } from '../services/client';
import { useAuth } from '../auth/AuthContext';
import { canWriteDomain } from '../services/auth';
import { createResource, deleteResource, getResource, listResource, updateResource, type Entity } from '../services/resources';

type FieldKind = 'text' | 'textarea' | 'number' | 'duration' | 'switch' | 'select' | 'secret';
interface SelectOptionSpec { label: string; value: string; }
interface FieldSpec { key: string; label: string; kind?: FieldKind; required?: boolean; options?: Array<string | SelectOptionSpec>; readonly?: boolean; defaultValue?: unknown; fullWidth?: boolean; min?: number; placeholder?: string; pattern?: RegExp; patternMessage?: string; preserveEmptyOnEdit?: boolean; }
interface ResourceSpec { title: string; description: string; path: string; idKey: string; fields: FieldSpec[]; detailPath?: string; createLabel?: string; readOnly?: boolean; action?: 'credit'; }

const valueText = (value: unknown) => value === null || value === undefined || value === '' ? '—' : String(value);
const moneyFields = new Set(['balance', 'credit_limit', 'price_per_interval', 'amount', 'balance_after', 'cost']);
const integerFields = new Set(['billing_interval_secs']);
const moneyText = (value: unknown) => {
  if (value === null || value === undefined || value === '') return '—';
  const amount = Number(value);
  if (!Number.isFinite(amount)) return String(value);
  return amount.toLocaleString('zh-CN', { minimumFractionDigits: 0, maximumFractionDigits: 3 });
};
const durationSecondsText = (value: unknown) => {
  if (value === null || value === undefined || value === '') return '—';
  const milliseconds = Number(value);
  if (!Number.isFinite(milliseconds)) return String(value);
  return (milliseconds / 1000).toLocaleString('zh-CN', { minimumFractionDigits: 0, maximumFractionDigits: 3 });
};
const entityId = (entity: Entity, key: string) => String(entity[key] ?? entity.id ?? '');
const statusColor = (value: unknown) => {
  const status = String(value ?? '').toLowerCase();
  if (['active', 'online', 'registered', 'healthy', 'answered', 'enabled', 'closed'].includes(status)) return 'green';
  if (['failed', 'offline', 'unhealthy', 'blocked', 'open', 'disabled'].includes(status)) return 'red';
  if (['draining', 'ringing', 'pending', 'half_open'].includes(status)) return 'orange';
  return 'arcoblue';
};

function PageHeader({ title, description, actions }: { title: string; description: string; actions?: ReactNode }) {
  return <header className="page-header"><div><h1>{title}</h1><p>{description}</p></div>{actions && <div className="page-actions">{actions}</div>}</header>;
}

function ErrorState({ error, retry }: { error: string; retry: () => void }) {
  return <Alert type="error" title="数据加载失败" content={error} action={<Button size="small" onClick={retry}>重试</Button>} />;
}

function FormControl({ field, disabled = false, value, onChange }: { field: FieldSpec; disabled?: boolean; value?: unknown; onChange: (value: unknown) => void }) {
  if (field.kind === 'number') return <InputNumber disabled={disabled} min={field.min ?? 0} precision={moneyFields.has(field.key) ? 3 : integerFields.has(field.key) ? 0 : undefined} placeholder={field.placeholder} value={value as number | undefined} onChange={onChange} style={{ width: '100%' }} />;
  if (field.kind === 'switch') return <Switch disabled={disabled} checked={Boolean(value)} onChange={onChange} />;
  if (field.kind === 'select') return <Select disabled={disabled} placeholder={field.placeholder} value={value as string | undefined} onChange={onChange} options={(field.options || []).map((option) => typeof option === 'string' ? { label: option, value: option } : option)} />;
  if (field.kind === 'secret') return <Input.Password disabled={disabled} placeholder={field.placeholder} value={String(value ?? '')} onChange={onChange} />;
  if (field.kind === 'textarea') return <Input.TextArea disabled={disabled} placeholder={field.placeholder} value={String(value ?? '')} onChange={onChange} autoSize={{ minRows: 3, maxRows: 7 }} />;
  return <Input disabled={disabled} placeholder={field.placeholder} value={String(value ?? '')} onChange={onChange} />;
}

export function resourceFormValues(spec: ResourceSpec, row: Entity | null): Entity {
  if (row) return { ...row };
  return spec.fields.reduce<Entity>((defaults, field) => {
    if (field.defaultValue !== undefined) defaults[field.key] = field.defaultValue;
    else if (field.kind === 'switch') defaults[field.key] = false;
    if (field.required && field.kind === 'select' && field.options?.[0]) {
      const option = field.options[0];
      defaults[field.key] = typeof option === 'string' ? option : option.value;
    }
    return defaults;
  }, {});
}

export function resourceSaveValues(spec: ResourceSpec, values: Entity, editing: boolean): Entity {
  if (!editing) return values;
  const result = { ...values };
  spec.fields.filter((field) => field.kind === 'secret' && field.preserveEmptyOnEdit).forEach((field) => {
    if (result[field.key] === '' || result[field.key] === undefined) delete result[field.key];
  });
  return result;
}

function ResourceWorkspace({ spec }: { spec: ResourceSpec }) {
  const [rows, setRows] = useState<Entity[]>([]);
  const [pagination, setPagination] = useState({ page: 1, page_size: 20, total: 0, total_pages: 0 });
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState<Entity | null | undefined>(undefined);
  const [draft, setDraft] = useState<Entity>({});
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const [actionRow, setActionRow] = useState<Entity | null>(null);
  const [amount, setAmount] = useState<number>(100);
  const navigate = useNavigate();
  const load = useCallback(async (page = pagination.page) => {
    setLoading(true); setError('');
    try {
      const result = await listResource(spec.path, { page, page_size: pagination.page_size });
      setRows(result.items || []); setPagination(result.pagination || { page, page_size: 20, total: result.items?.length || 0, total_pages: 1 });
    } catch (reason) { setError(reason instanceof Error ? reason.message : '加载失败'); }
    finally { setLoading(false); }
  }, [pagination.page, pagination.page_size, spec.path]);
  useEffect(() => { void load(1); }, [spec.path]);

  const isEditing = editing !== undefined && editing !== null;
  const openForm = (row: Entity | null) => {
    setDraft(resourceFormValues(spec, row));
    setValidationErrors({});
    setEditing(row);
  };
  const updateDraft = (key: string, value: unknown) => {
    setDraft((current) => ({ ...current, [key]: value }));
    setValidationErrors((current) => {
      if (!current[key]) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  };
  const save = async () => {
    try {
      const errors = spec.fields.reduce<Record<string, string>>((result, field) => {
        if (field.readonly || (isEditing && field.preserveEmptyOnEdit)) return result;
        const value = draft[field.key];
        const isEmpty = value === undefined || value === null || value === '';
        if (field.required && isEmpty) result[field.key] = `请填写${field.label}`;
        else if (!isEmpty && field.pattern && !field.pattern.test(String(value))) result[field.key] = field.patternMessage || `${field.label}格式不正确`;
        else if (!isEmpty && field.min !== undefined && Number(value) < field.min) result[field.key] = `${field.label}不能小于 ${field.min}`;
        return result;
      }, {});
      if (Object.keys(errors).length) { setValidationErrors(errors); return; }
      const values = resourceSaveValues(spec, draft, isEditing); setSaving(true);
      if (isEditing) await updateResource(spec.path, entityId(editing, spec.idKey), values);
      else await createResource(spec.path, values);
      Message.success(isEditing ? '已保存更改' : '已创建'); setEditing(undefined); await load();
    } catch (reason) { if (reason instanceof Error) Message.error(reason.message); }
    finally { setSaving(false); }
  };
  const remove = async (row: Entity) => {
    try { await deleteResource(spec.path, entityId(row, spec.idKey)); Message.success('已删除'); await load(); }
    catch (reason) { Message.error(reason instanceof Error ? reason.message : '删除失败'); }
  };
  const runAction = async () => {
    if (!actionRow || spec.action !== 'credit') return;
    try { setSaving(true); await api.post(`${spec.path}/${encodeURIComponent(entityId(actionRow, spec.idKey))}/credit`, { amount }); Message.success('充值成功'); setActionRow(null); await load(); }
    catch (reason) { Message.error(reason instanceof Error ? reason.message : '操作失败'); }
    finally { setSaving(false); }
  };
  const columns = [
    ...spec.fields.filter((field) => field.kind !== 'secret').slice(0, 7).map((field) => ({
      title: field.label, dataIndex: field.key, ellipsis: true,
      render: (value: unknown) => ['status', 'state', 'enabled', 'health'].includes(field.key)
        ? <Tag color={statusColor(value)}>{typeof value === 'boolean' ? (value ? '启用' : '停用') : valueText(value)}</Tag>
        : <span className={field.key.includes('id') || field.key.includes('number') ? 'mono' : ''}>{field.kind === 'duration' ? durationSecondsText(value) : moneyFields.has(field.key) ? moneyText(value) : valueText(value)}</span>,
    })),
    { title: '', width: spec.readOnly ? 74 : 142, fixed: 'right' as const, render: (_: unknown, row: Entity) => <Space>
      {spec.detailPath && <Button type="text" icon={<IconEye />} aria-label="查看详情" onClick={() => navigate(`${spec.detailPath}/${entityId(row, spec.idKey)}`)} />}
      {spec.action === 'credit' && <Button size="small" onClick={() => setActionRow(row)}>充值</Button>}
      {!spec.readOnly && <Button type="text" icon={<IconEdit />} aria-label="编辑" onClick={() => openForm(row)} />}
      {!spec.readOnly && <Popconfirm title="确认删除此资源？" onOk={() => remove(row)}><Button type="text" status="danger" icon={<IconDelete />} aria-label="删除" /></Popconfirm>}
    </Space> },
  ];
  const normalizedQuery = query.trim().toLowerCase();
  const visibleRows = normalizedQuery ? rows.filter((row) => Object.values(row).some((value) => String(value ?? '').toLowerCase().includes(normalizedQuery))) : rows;
  return <section className="workspace">
    <PageHeader title={spec.title} description={spec.description} actions={<Space>
      <Button icon={<IconRefresh />} onClick={() => load()} loading={loading}>刷新</Button>
      {!spec.readOnly && <Button type="primary" icon={<IconPlus />} onClick={() => openForm(null)}>{spec.createLabel || '新建'}</Button>}
    </Space>} />
    <div className="toolbar"><Input prefix={<IconSearch />} value={query} onChange={setQuery} allowClear placeholder={`筛选当前页${spec.title}`} /><span>{normalizedQuery ? `本页 ${visibleRows.length} 条` : `${pagination.total} 条记录`}</span></div>
    {error ? <ErrorState error={error} retry={() => load()} /> : <div className="data-table"><Table rowKey={(row) => entityId(row, spec.idKey)} loading={loading} columns={columns} data={visibleRows} pagination={false} scroll={{ x: 900 }} noDataElement={<Empty description="暂无数据" />} /></div>}
    {pagination.total > pagination.page_size && <Pagination current={pagination.page} pageSize={pagination.page_size} total={pagination.total} onChange={(page) => load(page)} showTotal />}
    <Modal className="resource-form-modal" style={{ width: 760 }} title={isEditing ? `编辑${spec.title}` : `新建${spec.title}`} visible={editing !== undefined} onCancel={() => setEditing(undefined)} onOk={save} confirmLoading={saving} unmountOnExit>
      <Form layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}>{spec.fields.filter((field) => !field.readonly).map((field) => {
        const error = validationErrors[field.key];
        return <Grid.Col key={field.key} xs={24} md={field.fullWidth ? 24 : 12}><Form.Item label={field.label} required={field.required && !(isEditing && field.preserveEmptyOnEdit)} validateStatus={error ? 'error' : undefined} help={error}><FormControl field={field} disabled={isEditing && field.key === spec.idKey} value={draft[field.key]} onChange={(value) => updateDraft(field.key, value)} /></Form.Item></Grid.Col>;
      })}</Grid.Row></Form>
    </Modal>
    <Modal title={`账户充值 · ${actionRow ? entityId(actionRow, spec.idKey) : ''}`} visible={Boolean(actionRow)} onCancel={() => setActionRow(null)} onOk={runAction} confirmLoading={saving} unmountOnExit><Form layout="vertical"><Form.Item label="充值金额"><InputNumber min={0.001} max={100000000} precision={3} value={amount} onChange={(value) => setAmount(Number(value))} style={{ width: '100%' }} /></Form.Item></Form></Modal>
  </section>;
}

const extensions: ResourceSpec = { title: '分机', description: '管理 SIP 身份、凭据状态和呼叫策略。', path: '/extensions', idKey: 'username', detailPath: '/extensions', createLabel: '新建分机', fields: [
  { key: 'username', label: '分机号', required: true }, { key: 'password', label: 'SIP 密码', kind: 'secret', required: true },
] };
const trunks: ResourceSpec = { title: 'SIP 中继', description: '配置运营商互联、容量和健康检查。', path: '/trunks', idKey: 'id', detailPath: '/trunks', createLabel: '新建中继', fields: [
  { key: 'id', label: '中继 ID', required: true, placeholder: '例如 carrier-cn-01' }, { key: 'host', label: '主机地址', required: true, placeholder: 'IP 地址或域名' }, { key: 'port', label: 'SIP 端口', kind: 'number', defaultValue: 5060 },
  { key: 'transport', label: '传输协议（TCP/TLS 待接入）', kind: 'select', required: true, options: [{ label: 'UDP', value: 'udp' }], placeholder: '当前仅支持 UDP' }, { key: 'gateway_type', label: '中继类型', kind: 'select', required: true, options: [{ label: '对等中继', value: 'peer' }, { label: '出局网关', value: 'gateway' }, { label: '注册终端', value: 'extension' }], defaultValue: 'peer' },
  { key: 'max_capacity', label: '容量上限', kind: 'number', defaultValue: 100 }, { key: 'max_concurrent', label: '单账户并发', kind: 'number', defaultValue: 100 }, { key: 'account_id', label: '计费账户 ID', kind: 'number', placeholder: '留空表示不关联计费账户' },
  { key: 'prefix_rules', label: '号码前缀改写', kind: 'textarea', fullWidth: true, placeholder: '格式：原前缀:新前缀，多个规则使用逗号分隔，例如 86:0086' },
  { key: 'caller_id_mode', label: '主叫号码策略', kind: 'select', required: true, options: [{ label: '透传主叫', value: 'passthrough' }, { label: '固定虚拟主叫', value: 'virtual' }, { label: '随机虚拟主叫', value: 'random' }], defaultValue: 'passthrough' }, { key: 'virtual_caller', label: '虚拟主叫号码', placeholder: '主叫策略为固定虚拟主叫时填写' },
  { key: 'supports_registration', label: '需要注册', kind: 'switch' }, { key: 'reg_auth_type', label: '注册认证', kind: 'select', required: true, options: [{ label: '无需认证', value: 'none' }, { label: 'IP 白名单', value: 'ip' }, { label: '用户名密码', value: 'digest' }], defaultValue: 'none' }, { key: 'reg_username', label: '注册用户名' }, { key: 'reg_password', label: '注册密码', kind: 'secret', placeholder: '编辑时留空表示不修改', preserveEmptyOnEdit: true },
  { key: 'circuit_state', label: '熔断状态', readonly: true }, { key: 'enabled', label: '启用', kind: 'switch', defaultValue: true },
] };
const numbers: ResourceSpec = { title: '号码库存', description: '将 DID 映射到已注册分机。', path: '/numbers', idKey: 'number', createLabel: '录入号码', fields: [
  { key: 'number', label: 'DID 号码', required: true }, { key: 'username', label: '目标分机', required: true }, { key: 'status', label: '状态', kind: 'select', required: true, options: ['available', 'assigned', 'disabled'] },
] };
const accounts: ResourceSpec = { title: '计费账户', description: '查看余额、币种和账户可用状态。', path: '/billing/accounts', idKey: 'username', readOnly: true, action: 'credit', fields: [
  { key: 'username', label: '账户' }, { key: 'balance', label: '余额', kind: 'number' }, { key: 'currency', label: '币种' }, { key: 'created_at', label: '创建时间' },
] };
const rates: ResourceSpec = { title: '费率', description: '按号码前缀配置计费周期与周期价格。', path: '/billing/rates', idKey: 'id', createLabel: '新建费率', fields: [
  { key: 'id', label: '费率 ID', required: true }, { key: 'prefix', label: '号码前缀', placeholder: '留空表示默认费率' }, { key: 'billing_interval_secs', label: '计费周期（秒）', kind: 'number', min: 1, required: true, defaultValue: 60 }, { key: 'price_per_interval', label: '周期价格（元）', kind: 'number', min: 0.001, required: true, defaultValue: 0.5 }, { key: 'description', label: '说明', kind: 'textarea', fullWidth: true },
] };
const transactions: ResourceSpec = { title: '账务流水', description: '按通话追踪扣费、余额变化和处理结果。', path: '/billing/transactions', idKey: 'id', readOnly: true, fields: [
  { key: 'id', label: '流水号' }, { key: 'call_id', label: '通话 ID' }, { key: 'username', label: '账户' }, { key: 'duration_ms', label: '计费时长（秒）', kind: 'duration' }, { key: 'billing_interval_secs', label: '计费周期（秒）' }, { key: 'price_per_interval', label: '周期价格（元）' }, { key: 'amount', label: '金额（元）' }, { key: 'balance_after', label: '余额（元）' }, { key: 'created_at', label: '发生时间' },
] };
const calls: ResourceSpec = { title: '通话记录', description: '查询呼叫结果、时长、路由和媒体质量。', path: '/calls', idKey: 'call_id', detailPath: '/calls', readOnly: true, fields: [
  { key: 'call_id', label: '通话 ID' }, { key: 'caller', label: '主叫' }, { key: 'callee', label: '被叫' }, { key: 'direction', label: '方向' }, { key: 'status', label: '结果' }, { key: 'duration_ms', label: '时长（秒）', kind: 'duration' }, { key: 'started_at_ms', label: '开始时间' },
] };

export const ExtensionsPage = () => <ResourceWorkspace spec={extensions} />;
export const TrunksPage = () => <ResourceWorkspace spec={trunks} />;
export const NumbersPage = () => <ResourceWorkspace spec={numbers} />;
export const AccountsPage = () => <ResourceWorkspace spec={accounts} />;
export const RatesPage = () => <ResourceWorkspace spec={rates} />;
export const TransactionsPage = () => <ResourceWorkspace spec={transactions} />;
export const CallsPage = () => <ResourceWorkspace spec={calls} />;

interface Summary { active_calls?: number; today_total_calls?: number; answer_rate?: number; registered_users?: number; active_gateways?: number; today_failed_calls?: number; }
export function DashboardPage() {
  const [data, setData] = useState<Summary>({}); const [error, setError] = useState(''); const [loading, setLoading] = useState(true);
  const load = useCallback(async () => { setLoading(true); setError(''); try { setData(await api.get<Summary>('/overview/summary')); } catch (e) { setError(e instanceof Error ? e.message : '加载失败'); } finally { setLoading(false); } }, []);
  useEffect(() => { void load(); }, [load]);
  return <section className="workspace"><PageHeader title="运行总览" description="当前软交换业务与节点的即时状态。" actions={<Button icon={<IconRefresh />} loading={loading} onClick={load}>刷新</Button>} />
    {error ? <ErrorState error={error} retry={load} /> : <Spin loading={loading} block><Grid.Row gutter={[12, 12]} className="metric-grid">
      {[['活跃通话', data.active_calls], ['今日呼叫', data.today_total_calls], ['接通率', data.answer_rate === undefined ? undefined : `${data.answer_rate}%`], ['在线分机', data.registered_users], ['可用中继', data.active_gateways], ['失败呼叫', data.today_failed_calls]].map(([label, value]) => <Grid.Col xs={12} md={8} xl={4} key={String(label)}><div className="metric"><span>{label}</span><strong>{valueText(value)}</strong></div></Grid.Col>)}
    </Grid.Row><div className="section-block"><h2>运行状态</h2><p className="muted-copy">总览按需刷新；活跃通话页面每 10 秒同步一次实时控制面状态。</p></div></Spin>}
  </section>;
}

export function ActiveCallsPage() {
  const [rows, setRows] = useState<Entity[]>([]); const [loading, setLoading] = useState(true); const [error, setError] = useState(''); const navigate = useNavigate();
  const { session } = useAuth();
  const load = useCallback(async () => { setLoading(true); setError(''); try { setRows(await api.get<Entity[]>('/calls/active')); } catch (e) { setError(e instanceof Error ? e.message : '加载失败'); } finally { setLoading(false); } }, []);
  useEffect(() => { void load(); const timer = window.setInterval(load, 10000); return () => window.clearInterval(timer); }, [load]);
  const terminate = async (row: Entity) => { try { await api.post(`/calls/${encodeURIComponent(entityId(row, 'call_id'))}/actions/terminate`); Message.success('已发送挂断指令'); await load(); } catch (e) { Message.error(e instanceof Error ? e.message : '操作失败'); } };
  return <section className="workspace"><PageHeader title="活跃通话" description="实时查看正在建立和已接通的会话。" actions={<Button icon={<IconRefresh />} loading={loading} onClick={load}>刷新</Button>} />{error ? <ErrorState error={error} retry={load} /> : <div className="data-table"><Table loading={loading} pagination={false} rowKey="id" data={rows} columns={[
    { title: '通话 ID', dataIndex: 'call_id', render: (v) => <span className="mono">{v}</span> }, { title: '主叫', dataIndex: 'caller' }, { title: '被叫', dataIndex: 'callee' }, { title: '状态', dataIndex: 'state', render: (v) => <Tag color={statusColor(v)}>{v}</Tag> }, { title: '开始时间', dataIndex: 'started_at_ms' }, { title: '中继', dataIndex: 'gateway' },
    { title: '', render: (_, row) => <Space><Button type="text" icon={<IconEye />} onClick={() => navigate(`/calls/${entityId(row, 'call_id')}`)} />{session && canWriteDomain(session.role, 'operations') && <Popconfirm title="确认挂断此通话？" onOk={() => terminate(row)}><Button status="danger" size="small">挂断</Button></Popconfirm>}</Space> },
  ]} /></div>}</section>;
}

function detailValue(value: unknown, key?: string): ReactNode {
  if (value === null || value === undefined || value === '') return '—';
  if (key?.endsWith('duration_ms')) return `${durationSecondsText(value)} 秒`;
  if (key && moneyFields.has(key)) return moneyText(value);
  if (typeof value === 'boolean') return value ? '是' : '否';
  if (Array.isArray(value)) return `${value.length} 项`;
  if (typeof value === 'object') return '查看关联状态';
  return String(value);
}

function DetailFields({ value, empty }: { value: unknown; empty: string }) {
  if (!value || typeof value !== 'object') return <Empty description={empty} />;
  return <Descriptions column={2} border data={Object.entries(value as Entity).map(([label, fieldValue]) => ({ label, value: detailValue(fieldValue, label) }))} />;
}

function CallSummary({ entity }: { entity: Entity }) {
  const availability = String(entity.runtime_availability ?? 'unavailable');
  return <div className="detail-sections">
    <div className="section-block"><div className="section-title"><h2>历史通话</h2><Tag color={entity.historical ? 'green' : 'gray'}>{entity.historical ? '已持久化' : '暂无 CDR'}</Tag></div><DetailFields value={entity.historical} empty="暂无历史通话数据" /></div>
    <div className="section-block"><div className="section-title"><h2>实时状态</h2><Tag color={availability === 'available' ? 'green' : availability === 'not_active' ? 'gray' : 'orange'}>{availability}</Tag></div><DetailFields value={entity.runtime} empty={availability === 'not_active' ? '通话已结束' : '实时控制面不可用'} /></div>
  </div>;
}

function EntityDetail({ path, title, rootKey, tabs }: { path: string; title: string; rootKey?: string; tabs: { key: string; title: string; path?: string; sourceKey?: string }[] }) {
  const { id = '' } = useParams(); const [entity, setEntity] = useState<Entity | null>(null); const [related, setRelated] = useState<Record<string, unknown>>({}); const [loading, setLoading] = useState(true); const [error, setError] = useState('');
  const load = useCallback(async () => { setLoading(true); setError(''); try { setEntity(await getResource(path, id)); } catch (e) { setError(e instanceof Error ? e.message : '加载失败'); } finally { setLoading(false); } }, [id, path]);
  useEffect(() => { void load(); }, [load]);
  const loadTab = async (key: string, subpath?: string, sourceKey?: string) => { if (related[key] !== undefined) return; if (sourceKey && entity) { setRelated((old) => ({ ...old, [key]: entity[sourceKey] })); return; } if (!subpath) return; try { const value = subpath === 'recording' ? URL.createObjectURL(await api.blob(`${path}/${encodeURIComponent(id)}/${subpath}`)) : await api.get(`${path}/${encodeURIComponent(id)}/${subpath}`); setRelated((old) => ({ ...old, [key]: value })); } catch (e) { setRelated((old) => ({ ...old, [key]: { error: e instanceof Error ? e.message : '加载失败' } })); } };
  const renderObject = (value: unknown) => { if (value === undefined) return <Spin />; if (typeof value === 'string' && value.startsWith('blob:')) return <audio className="recording-player" controls src={value} />; if (value && typeof value === 'object' && 'error' in value) return <Alert type="error" content={String((value as Entity).error)} />; const list = Array.isArray(value) ? value : [value]; return <div className="related-list">{list.map((item, index) => <DetailFields key={index} value={item} empty="暂无数据" />)}</div>; };
  const root = rootKey && entity?.[rootKey] && typeof entity[rootKey] === 'object' ? entity[rootKey] as Entity : entity;
  return <section className="workspace"><PageHeader title={root ? valueText(root.name || root.username || root.id || id) : title} description={`${title}详情与关联运行状态。`} actions={<Button icon={<IconRefresh />} onClick={load}>刷新</Button>} />{error ? <ErrorState error={error} retry={load} /> : <Spin loading={loading} block>{entity && <Tabs defaultActiveTab="summary" onChange={(key) => { const tab = tabs.find((t) => t.key === key); void loadTab(key, tab?.path, tab?.sourceKey); }}>{tabs.map((tab) => <Tabs.TabPane key={tab.key} title={tab.title}>{tab.key === 'summary' && path === '/calls' ? <CallSummary entity={entity} /> : tab.key === 'summary' && root ? <DetailFields value={root} empty="暂无详情" /> : renderObject(related[tab.key])}</Tabs.TabPane>)}</Tabs>}</Spin>}</section>;
}
export const ExtensionDetailPage = () => <EntityDetail path="/extensions" title="分机" rootKey="extension" tabs={[{ key: 'summary', title: '概览' }, { key: 'registrations', title: '注册终端', sourceKey: 'registrations' }, { key: 'numbers', title: '号码', sourceKey: 'numbers' }, { key: 'credential', title: '凭据', sourceKey: 'credential' }]} />;
export const TrunkDetailPage = () => <EntityDetail path="/trunks" title="中继" rootKey="trunk" tabs={[{ key: 'summary', title: '概览' }, { key: 'health', title: '健康状态', sourceKey: 'health' }, { key: 'numbers', title: '号码', sourceKey: 'numbers' }, { key: 'routes', title: '依赖路由', sourceKey: 'routes' }]} />;
export const CallDetailPage = () => <EntityDetail path="/calls" title="通话" tabs={[{ key: 'summary', title: '呼叫概览' }, { key: 'media', title: '媒体指标', path: 'media' }, { key: 'dtmf', title: 'DTMF', path: 'dtmf' }, { key: 'recording', title: '录音', path: 'recording' }]} />;

export function RoutesPage() {
  const routeSpec: ResourceSpec = useMemo(() => ({ title: '路由策略', description: '按优先级匹配、改写号码并选择中继。', path: '/routing/rules', idKey: 'id', createLabel: '新建规则', fields: [
    { key: 'id', label: '规则 ID', required: true, placeholder: '例如 cn-mobile-primary' }, { key: 'prefix', label: '匹配前缀', placeholder: '留空表示匹配全部号码' }, { key: 'priority', label: '优先级', kind: 'number', required: true, defaultValue: 100 }, { key: 'gateway_id', label: '目标中继', required: true, placeholder: '填写已存在的中继 ID' }, { key: 'cost', label: '路由成本', kind: 'number', required: true, defaultValue: 0 }, { key: 'weight', label: '分流权重', kind: 'number', min: 1, defaultValue: 100 },
    { key: 'time_start', label: '生效开始', placeholder: 'HH:MM，例如 08:00', pattern: /^([01]\d|2[0-3]):[0-5]\d$/, patternMessage: '请输入 HH:MM 格式的时间' }, { key: 'time_end', label: '生效结束', placeholder: 'HH:MM，例如 22:00', pattern: /^([01]\d|2[0-3]):[0-5]\d$/, patternMessage: '请输入 HH:MM 格式的时间' },
  ] }), []);
  const [open, setOpen] = useState(false); const [loading, setLoading] = useState(false); const [result, setResult] = useState<Entity | null>(null); const [form] = Form.useForm();
  const simulate = async () => { try { const values = await form.validate(); setLoading(true); setResult(await api.get<Entity>('/routing/simulations', values)); } catch (e) { if (e instanceof Error) Message.error(e.message); } finally { setLoading(false); } };
  return <><ResourceWorkspace spec={routeSpec} /><Button className="floating-action" type="primary" icon={<IconSend />} onClick={() => setOpen(true)}>路由仿真</Button><Drawer title="路由仿真" width={560} visible={open} onCancel={() => setOpen(false)} footer={<Button type="primary" loading={loading} onClick={simulate}>执行仿真</Button>}><Form form={form} layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}><Grid.Col xs={24} md={24}><Form.Item field="destination" label="目标号码" rules={[{ required: true, message: '请输入目标号码' }]}><Input /></Form.Item></Grid.Col></Grid.Row></Form>{result && <div className="simulation-result"><h3>匹配结果</h3><pre>{JSON.stringify(result, null, 2)}</pre></div>}</Drawer></>;
}

export function SecurityPage() {
  const rules: ResourceSpec = { title: '安全策略', description: '管理防盗打规则并查看近期风险事件。', path: '/security/anti-fraud/policies', idKey: 'id', createLabel: '新建策略', fields: [
    { key: 'id', label: '策略 ID', required: true }, { key: 'rule_type', label: '规则类型', kind: 'select', required: true, options: ['cps_limit', 'concurrent_limit', 'number_blocklist', 'ip_blocklist'] }, { key: 'target_value', label: '目标值', kind: 'textarea', required: true, fullWidth: true }, { key: 'limit_number', label: '阈值', kind: 'number' }, { key: 'enabled', label: '启用', kind: 'switch', defaultValue: true },
  ] };
  return <ResourceWorkspace spec={rules} />;
}

export function InfrastructurePage() {
  const [sip, setSip] = useState<Entity>({}); const [media, setMedia] = useState<Entity>({}); const [metrics, setMetrics] = useState<Entity>({}); const [loading, setLoading] = useState(true); const [error, setError] = useState('');
  const load = useCallback(async () => { setLoading(true); setError(''); try { const [sipData, mediaData, metricData] = await Promise.all([api.get<Entity>('/infrastructure/sip-cluster'), api.get<Entity>('/infrastructure/media-cluster'), api.get<Entity>('/infrastructure/media/metrics')]); setSip(sipData); setMedia(mediaData); setMetrics(metricData); } catch (e) { setError(e instanceof Error ? e.message : '加载失败'); } finally { setLoading(false); } }, []);
  useEffect(() => { void load(); }, [load]);
  const control = async (id: string, action: 'drain' | 'resume') => { try { await api.post(`/infrastructure/sip-cluster/nodes/${encodeURIComponent(id)}/${action}`); Message.success(action === 'drain' ? '节点已摘流' : '节点已恢复'); await load(); } catch (e) { Message.error(e instanceof Error ? e.message : '操作失败'); } };
  const sipNodes = Array.isArray(sip.nodes) ? sip.nodes as Entity[] : [];
  return <section className="workspace"><PageHeader title="集群节点" description="查看 SIP 与媒体资源状态并执行节点摘流。" actions={<Button icon={<IconRefresh />} loading={loading} onClick={load}>刷新</Button>} />{error ? <ErrorState error={error} retry={load} /> : <Spin loading={loading} block><Tabs defaultActiveTab="sip"><Tabs.TabPane key="sip" title="SIP 集群"><div className="data-table"><Table pagination={false} rowKey="node_id" data={sipNodes} columns={[{ title: '节点', dataIndex: 'node_id' }, { title: '通告地址', dataIndex: 'advertised_addr' }, { title: '状态', dataIndex: 'status', render: (v) => <Tag color={statusColor(v)}>{v}</Tag> }, { title: '活跃通话', dataIndex: 'active_calls' }, { title: '版本', dataIndex: 'version' }, { title: '', render: (_, row) => row.status === 'draining' ? <Button size="small" onClick={() => control(String(row.node_id), 'resume')}>恢复</Button> : <Popconfirm title="摘流后节点将不接收新呼叫，确认继续？" onOk={() => control(String(row.node_id), 'drain')}><Button size="small">摘流</Button></Popconfirm> }]} /></div></Tabs.TabPane><Tabs.TabPane key="media" title="媒体集群"><pre>{JSON.stringify(media, null, 2)}</pre></Tabs.TabPane><Tabs.TabPane key="metrics" title="媒体指标"><pre>{JSON.stringify(metrics, null, 2)}</pre></Tabs.TabPane></Tabs></Spin>}</section>;
}

type ConfigKind = 'text' | 'number' | 'decimal' | 'boolean' | 'secret';
interface ConfigField { key: string; label: string; kind?: ConfigKind; hint: string; fullWidth?: boolean; }
interface ConfigGroup { key: string; label: string; description: string; fields: ConfigField[]; }

const systemConfigGroups: ConfigGroup[] = [
  { key: 'sip', label: 'SIP 与会话', description: '认证域与呼叫会话计时器。', fields: [
    { key: 'realm', label: '认证 Realm', hint: 'Digest 认证域；存在分机时不可修改' },
    { key: 'session_expires_gateway', label: '网关会话时长', kind: 'number', hint: '单位：秒' },
    { key: 'session_expires_caller', label: '主叫会话时长', kind: 'number', hint: '单位：秒' },
  ] },
  { key: 'routing', label: '路由与中继', description: '路由运行依赖的中继健康探测。', fields: [
    { key: 'gateway_health_checks_enabled', label: '中继健康检查', kind: 'boolean', hint: '定期探测中继可用状态' },
  ] },
  { key: 'media', label: '媒体', description: 'RTP 地址学习、防欺骗与质量指标。', fields: [
    { key: 'rtp_symmetric_learning', label: '对称 RTP 学习', kind: 'boolean', hint: '从首个有效媒体包学习源地址' },
    { key: 'rtp_anti_spoofing', label: 'RTP 防欺骗', kind: 'boolean', hint: '拒绝非预期媒体源' },
    { key: 'rtp_source_relearn_secs', label: '媒体源重新学习窗口', kind: 'number', hint: '单位：秒' },
    { key: 'media_metrics_log', label: '媒体指标日志', kind: 'boolean', hint: '输出媒体质量统计日志' },
  ] },
  { key: 'recording', label: '录音', description: '录音任务、存储容量与文件生命周期。', fields: [
    { key: 'recording_enabled', label: '启用录音', kind: 'boolean', hint: '允许系统创建通话录音' },
    { key: 'recording_dir', label: '录音目录', hint: '节点本地录音文件根目录', fullWidth: true },
    { key: 'recording_workers', label: '录音工作线程', kind: 'number', hint: '异步落盘工作线程数' },
    { key: 'recording_queue_capacity', label: '录音队列容量', kind: 'number', hint: '等待写入的任务上限' },
    { key: 'recording_retention_secs', label: '录音保留时长', kind: 'number', hint: '单位：秒' },
    { key: 'recording_min_free_bytes', label: '最小磁盘余量', kind: 'number', hint: '单位：字节' },
    { key: 'recording_max_file_bytes', label: '单文件上限', kind: 'number', hint: '单位：字节' },
    { key: 'recording_max_duration_secs', label: '单次录音时长上限', kind: 'number', hint: '单位：秒' },
  ] },
  { key: 'billing', label: '计费与 CDR', description: '余额风控、结算与话单持久化。', fields: [
    { key: 'balance_enforcement_enabled', label: '余额强制校验', kind: 'boolean', hint: '呼叫前校验账户可用余额' },
    { key: 'billing_settlement_enabled', label: '启用计费结算', kind: 'boolean', hint: '通话结束后执行费用结算' },
    { key: 'cdr_persistence_enabled', label: 'CDR 持久化', kind: 'boolean', hint: '写入通话详单存储' },
    { key: 'cdr_queue_capacity', label: 'CDR 队列容量', kind: 'number', hint: '等待持久化的话单上限' },
  ] },
  { key: 'security', label: 'SBC 与 TLS', description: '边界限流及 SIP TLS 连接安全。', fields: [
    { key: 'sbc_rate_limit_capacity', label: '令牌桶容量', kind: 'decimal', hint: '单一来源允许的突发请求量' },
    { key: 'sbc_rate_limit_fill_rate', label: '令牌补充速率', kind: 'decimal', hint: '每秒补充令牌数' },
    { key: 'sbc_max_concurrency', label: 'SBC 最大并发', kind: 'number', hint: '边界层并发会话上限' },
    { key: 'tls_bind_addr', label: 'TLS 监听地址', hint: '例如 0.0.0.0:5061' },
    { key: 'tls_cert_path', label: 'TLS 证书路径', hint: 'PEM 证书文件路径', fullWidth: true },
    { key: 'tls_key_path', label: 'TLS 私钥路径', hint: 'PEM 私钥文件路径', fullWidth: true },
    { key: 'tls_ca_path', label: 'TLS CA 路径', hint: '可信 CA 文件路径', fullWidth: true },
    { key: 'tls_server_name', label: 'TLS 服务名称', hint: '证书校验使用的服务名称' },
    { key: 'tls_allow_test_certificate', label: '允许测试证书', kind: 'boolean', hint: '仅用于测试环境' },
    { key: 'tls_insecure_skip_verify', label: '跳过证书校验', kind: 'boolean', hint: '高风险，仅用于隔离测试环境' },
  ] },
  { key: 'cluster', label: '节点运行', description: 'UDP 工作线程、套接字缓冲与节点密钥。', fields: [
    { key: 'udp_workers_auto', label: '自动分配 UDP Worker', kind: 'boolean', hint: '按 CPU 核心数决定工作线程' },
    { key: 'udp_workers', label: 'UDP Worker 数量', kind: 'number', hint: '关闭自动分配时生效' },
    { key: 'udp_receive_buffer_bytes', label: 'UDP 接收缓冲区', kind: 'number', hint: '单位：字节' },
    { key: 'udp_send_buffer_bytes', label: 'UDP 发送缓冲区', kind: 'number', hint: '单位：字节' },
    { key: 'secret_key', label: '节点密钥', kind: 'secret', hint: '留空表示不修改现有密钥', fullWidth: true },
  ] },
];

function ConfigControl({ field, value, onChange }: { field: ConfigField; value: unknown; onChange: (value: unknown) => void }) {
  if (field.kind === 'boolean') return <Select value={String(value ?? '')} onChange={onChange} options={[{ label: '启用', value: 'true' }, { label: '停用', value: 'false' }]} />;
  const numericValue = value === '' || value === undefined ? undefined : Number(value);
  if (field.kind === 'decimal') return <InputNumber value={numericValue} min={0} onChange={onChange} style={{ width: '100%' }} />;
  if (field.kind === 'number') return <InputNumber value={numericValue} min={0} precision={0} onChange={onChange} style={{ width: '100%' }} />;
  if (field.kind === 'secret') return <Input.Password value={String(value ?? '')} onChange={onChange} placeholder="••••••••" />;
  return <Input value={String(value ?? '')} onChange={onChange} />;
}

export function SettingsPage() {
  const [loading, setLoading] = useState(true); const [saving, setSaving] = useState(false); const [error, setError] = useState(''); const [configValues, setConfigValues] = useState<Entity>({});
  const load = useCallback(async () => { setLoading(true); setError(''); try { const result = await api.get<{ values: Entity }>('/infrastructure/settings'); const configs = result.values.configs; setConfigValues(configs && typeof configs === 'object' ? configs as Entity : result.values); } catch (e) { setError(e instanceof Error ? e.message : '加载失败'); } finally { setLoading(false); } }, []);
  useEffect(() => { void load(); }, [load]);
  const updateValue = (key: string, value: unknown) => setConfigValues((current) => ({ ...current, [key]: value }));
  const save = async () => { try { const payload = Object.fromEntries(Object.entries(configValues).filter(([key, value]) => value !== undefined && value !== null && !(key === 'secret_key' && !value)).map(([key, value]) => [key, String(value)])); setSaving(true); await api.post('/infrastructure/settings', payload); Message.success('设置已保存，重启节点后生效'); } catch (e) { if (e instanceof Error) Message.error(e.message); } finally { setSaving(false); } };
  return <section className="workspace"><PageHeader title="系统设置" description="管理核心运行参数。修改会同步至配置存储，节点重启后应用。" actions={<Space><Button icon={<IconRefresh />} disabled={saving} onClick={load}>重新加载</Button><Button type="primary" loading={saving} onClick={save}>保存设置</Button></Space>} />{error ? <ErrorState error={error} retry={load} /> : <Spin loading={loading} block><Alert className="settings-notice" type="warning" title="配置保存后需要重启相关节点" content="保存操作不会中断当前通话；请在维护窗口内逐节点重启并验证注册、路由与媒体状态。" />{!loading && <Form className="settings-form" layout="vertical"><Tabs defaultActiveTab="sip" type="line">{systemConfigGroups.map((group) => <Tabs.TabPane key={group.key} title={group.label}><div className="config-group-header"><div><h2>{group.label}</h2><p>{group.description}</p></div><Tag color="orange">重启生效</Tag></div><Grid.Row className="form-grid" gutter={[18, 0]}>{group.fields.map((field) => <Grid.Col key={field.key} xs={24} md={field.fullWidth ? 24 : 12}><Form.Item label={field.label} extra={<span className="config-hint">{field.hint}</span>}><ConfigControl field={field} value={configValues[field.key]} onChange={(value) => updateValue(field.key, value)} /></Form.Item></Grid.Col>)}</Grid.Row></Tabs.TabPane>)}</Tabs></Form>}</Spin>}</section>;
}
