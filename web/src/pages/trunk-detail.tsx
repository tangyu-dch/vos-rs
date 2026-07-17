import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Alert, Button, Empty, Form, Grid, Input, InputNumber, Message, Select, Space, Spin,
  Switch, Table, Tabs, Tag,
} from '@arco-design/web-react';
import { IconDelete, IconPlus, IconRefresh, IconSave } from '@arco-design/web-react/icon';
import { useParams } from 'react-router-dom';
import type { Entity } from '../services/resources';
import {
  getOutboundPolicy, getTrunkIpRules, getTrunkWorkspace, listOptions, saveOutboundPolicy,
  policyValidationError, saveTrunkIpRules, trunkRole, updateTrunk, type OutboundPolicy, type TrunkIpRule,
  type TrunkWorkspaceData, getTrunkEgressEndpoints, saveTrunkEgressEndpoints, type EgressEndpoint,
} from '../services/trunks';

export const emptyPolicy: OutboundPolicy = {
  caller_mode: 'strict_passthrough', fallback_mode: 'reject',
  egress_mode: 'direct', enabled: true,
};

const roleOptions = [{ label: '接入中继', value: 'access' }, { label: '落地中继', value: 'egress' }];
const authOptions = [
  { label: 'IP 白名单', value: 'ip_allowlist' },
  { label: '注册认证', value: 'digest_register' },
  { label: 'IP 加认证', value: 'ip_and_digest' },
];
const callerOptions = [
  { label: '严格透传', value: 'strict_passthrough' },
  { label: '固定号码', value: 'fixed_number' },
  { label: '虚拟主叫', value: 'virtual_pool' },
];

const genId = () => crypto.randomUUID ? crypto.randomUUID() : Math.random().toString(36).substring(2);

function Header({ title, loading, saving, onRefresh, onSave }: { title: string; loading: boolean; saving: boolean; onRefresh: () => void; onSave: () => void }) {
  return <header className="page-header"><div><h1>{title}</h1><p>中继身份、认证、主叫号码与落地范围配置。</p></div><Space><Button icon={<IconRefresh />} loading={loading} onClick={onRefresh}>刷新</Button><Button type="primary" icon={<IconSave />} loading={saving} onClick={onSave}>保存配置</Button></Space></header>;
}

export function WorkspaceField({ label, children, fullWidth = false, required = false }: { label: string; children: ReactNode; fullWidth?: boolean; required?: boolean }) {
  return <Grid.Col xs={24} md={fullWidth ? 24 : 12}><Form.Item label={label} required={required}>{children}</Form.Item></Grid.Col>;
}
const Field = WorkspaceField;

function BasicTab({ draft, set }: { draft: Entity; set: (key: string, value: unknown) => void }) {
  const role = trunkRole(draft);
  return <Form layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}>
    <WorkspaceField label="中继标识" required><Input value={String(draft.id ?? '')} disabled /></WorkspaceField>
    <Field label="中继类型" required><Select value={role} options={roleOptions} onChange={(value) => set('role', value)} /></Field>
    <Field label="计费账户"><InputNumber value={draft.account_id as number | undefined} onChange={(value) => set('account_id', value)} placeholder="可选" style={{ width: '100%' }} /></Field>
    {role === 'egress' && <><Field label="主机地址" required><Input value={String(draft.host ?? '')} onChange={(value) => set('host', value)} /></Field><Field label="SIP 端口" required><InputNumber value={Number(draft.port ?? 5060)} min={1} max={65535} onChange={(value) => set('port', value)} style={{ width: '100%' }} /></Field></>}
    <Field label="容量上限"><InputNumber value={Number(draft.max_capacity ?? 100)} min={0} onChange={(value) => set('max_capacity', value)} style={{ width: '100%' }} /></Field>
    <Field label="启用状态"><Switch checked={draft.enabled !== false} onChange={(value) => set('enabled', value)} checkedText="启用" uncheckedText="停用" /></Field>
  </Grid.Row></Form>;
}

function IpRulesEditor({ rules, onChange }: { rules: TrunkIpRule[]; onChange: (rules: TrunkIpRule[]) => void }) {
  const patch = (index: number, values: Partial<TrunkIpRule>) => onChange(rules.map((rule, itemIndex) => itemIndex === index ? { ...rule, ...values } : rule));
  const columns = [
    { title: '来源 IP/CIDR', render: (_: unknown, row: TrunkIpRule, index: number) => <Input value={row.cidr} onChange={(value) => patch(index, { cidr: value })} placeholder="例如 192.0.2.10/32" /> },
    { title: '来源端口', width: 140, render: (_: unknown, row: TrunkIpRule, index: number) => <InputNumber value={row.source_port ?? undefined} min={1} max={65535} onChange={(value) => patch(index, { source_port: value || null })} placeholder="任意" /> },
    { title: '传输协议', width: 110, render: () => <Select value="udp" options={[{ label: 'UDP', value: 'udp' }]} disabled /> },
    { title: '备注说明', render: (_: unknown, row: TrunkIpRule, index: number) => <Input value={row.description} onChange={(value) => patch(index, { description: value })} /> },
    { title: '启用', width: 74, render: (_: unknown, row: TrunkIpRule, index: number) => <Switch checked={row.enabled} onChange={(value) => patch(index, { enabled: value })} /> },
    { title: '', width: 58, render: (_: unknown, __: TrunkIpRule, index: number) => <Button type="text" status="danger" icon={<IconDelete />} aria-label="删除 IP 规则" onClick={() => onChange(rules.filter((_, itemIndex) => itemIndex !== index))} /> },
  ];
  return <div className="repeat-editor"><div className="section-title"><div><h2>来源地址</h2><p className="muted-copy">支持 IPv4、IPv6 和 CIDR；端口留空表示任意来源端口。</p></div><Button icon={<IconPlus />} onClick={() => onChange([...rules, { _key: genId(), cidr: '', source_port: null, transport: 'udp', description: '', enabled: true }])}>添加地址</Button></div><Table rowKey={(record) => record._key || record.id || record.cidr} pagination={false} data={rules} columns={columns} scroll={{ x: 820 }} noDataElement={<Empty description="尚未配置 IP 白名单" />} /></div>;
}

function EgressEndpointsEditor({ endpoints, onChange }: { endpoints: EgressEndpoint[]; onChange: (endpoints: EgressEndpoint[]) => void }) {
  const patch = (index: number, values: Partial<EgressEndpoint>) => onChange(endpoints.map((ep, itemIndex) => itemIndex === index ? { ...ep, ...values } : ep));
  const columns = [
    { title: '落地主机/IP', render: (_: unknown, row: EgressEndpoint, index: number) => <Input value={row.host} onChange={(value) => patch(index, { host: value })} placeholder="例如 203.0.113.50 或 sip.carrier.com" /> },
    { title: 'SIP 端口', width: 140, render: (_: unknown, row: EgressEndpoint, index: number) => <InputNumber value={row.port ?? undefined} min={1} max={65535} onChange={(value) => patch(index, { port: value || null })} placeholder="5060" /> },
    { title: '传输协议', width: 110, render: () => <Select value="udp" options={[{ label: 'UDP', value: 'udp' }]} disabled /> },
    { title: '优先级', width: 120, render: (_: unknown, row: EgressEndpoint, index: number) => <InputNumber value={row.priority ?? 100} min={0} max={65535} onChange={(value) => patch(index, { priority: value ?? 100 })} /> },
    { title: '启用', width: 74, render: (_: unknown, row: EgressEndpoint, index: number) => <Switch checked={row.enabled} onChange={(value) => patch(index, { enabled: value })} /> },
    { title: '', width: 58, render: (_: unknown, __: EgressEndpoint, index: number) => <Button type="text" status="danger" icon={<IconDelete />} aria-label="删除落地端点" onClick={() => onChange(endpoints.filter((_, itemIndex) => itemIndex !== index))} /> },
  ];
  return <div className="repeat-editor"><div className="section-title"><div><h2>落地端点 (Egress Endpoints)</h2><p className="muted-copy">静态直连中继可以配置多个落地服务器端点，支持优先级故障切换。</p></div><Button icon={<IconPlus />} onClick={() => onChange([...endpoints, { _key: genId(), host: '', port: 5060, transport: 'udp', priority: 100, enabled: true }])}>添加端点</Button></div><Table rowKey={(record) => record._key || record.id || record.host} pagination={false} data={endpoints} columns={columns} scroll={{ x: 820 }} noDataElement={<Empty description="尚未配置落地端点" />} /></div>;
}

function AccessAuthTab({ draft, set, rules, setRules }: { draft: Entity; set: (key: string, value: unknown) => void; rules: TrunkIpRule[]; setRules: (rules: TrunkIpRule[]) => void }) {
  const mode = String(draft.access_auth_mode ?? 'ip_allowlist');
  const showIp = mode === 'ip_allowlist' || mode === 'ip_and_digest';
  const showDigest = mode === 'digest_register' || mode === 'ip_and_digest';
  return <Form layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}>
    <Field label="认证方式" required><Select value={mode} options={authOptions} onChange={(value) => set('access_auth_mode', value)} /></Field>
    <Field label="认证 Realm"><Input value="继承系统 Realm" disabled /></Field>
    {showDigest && <><Field label="注册用户" required><Input value={String(draft.reg_username ?? '')} onChange={(value) => set('reg_username', value)} /></Field><Field label="注册密码"><Input.Password value={String(draft.reg_password ?? '')} onChange={(value) => set('reg_password', value)} placeholder="编辑时留空表示不修改" /></Field></>}
    {showIp && <Field label="IP 白名单" fullWidth><IpRulesEditor rules={rules} onChange={setRules} /></Field>}
  </Grid.Row></Form>;
}

function RegistrationTab({ draft, set, registrations, endpoints, setEndpoints }: { draft: Entity; set: (key: string, value: unknown) => void; registrations: Entity[]; endpoints: EgressEndpoint[]; setEndpoints: (endpoints: EgressEndpoint[]) => void }) {
  const mode = String(draft.egress_connection_type ?? 'static_peer');
  return <div className="detail-sections"><Form layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}>
    <Field label="连接方式" required><Select value={mode} options={[{ label: 'IP 直连', value: 'static_peer' }, { label: '主动注册', value: 'client_register', disabled: true }]} onChange={(value) => set('egress_connection_type', value)} /></Field>
    <Field label="传输协议" required><Select value="udp" options={[{ label: 'UDP', value: 'udp' }]} disabled /></Field>
    {mode === 'client_register' && <><Field label="注册服务器" required><Input value={String(draft.register_server ?? '')} onChange={(value) => set('register_server', value)} /></Field><Field label="注册用户" required><Input value={String(draft.register_username ?? '')} onChange={(value) => set('register_username', value)} /></Field><Field label="注册密码" required={!draft.has_register_password}><Input.Password value={String(draft.register_password ?? '')} onChange={(value) => set('register_password', value)} placeholder={draft.has_register_password ? '留空表示不修改' : ''} /></Field><Field label="注册周期"><InputNumber value={Number(draft.register_refresh_secs ?? 300)} min={60} onChange={(value) => set('register_refresh_secs', value)} style={{ width: '100%' }} /></Field></>}
  </Grid.Row></Form>
  {mode === 'static_peer' && <div style={{ marginBottom: 24 }}><EgressEndpointsEditor endpoints={endpoints} onChange={setEndpoints} /></div>}
  <div className="section-block"><div className="section-title"><h2>注册状态</h2><Tag color={registrations.length ? 'green' : 'gray'}>{registrations.length ? '已有注册' : '暂无注册'}</Tag></div>{registrations.length ? <Table pagination={false} data={registrations} columns={[{ title: '联系地址', dataIndex: 'contact' }, { title: '所在节点', dataIndex: 'node' }, { title: '过期时间', dataIndex: 'expires_at' }]} /> : <Empty description="暂无注册终端" />}</div></div>;
}

function AccessRegistrationStatus({ registrations }: { registrations: Entity[] }) {
  return <div className="section-block"><div className="section-title"><div><h2>注册终端</h2><p className="muted-copy">仅注册认证或 IP 加认证模式会产生第三方注册状态。</p></div><Tag color={registrations.length ? 'green' : 'gray'}>{registrations.length ? `${registrations.length} 个在线注册` : '暂无注册'}</Tag></div>{registrations.length ? <Table pagination={false} data={registrations} columns={[{ title: '联系地址', dataIndex: 'contact' }, { title: '所在节点', dataIndex: 'node' }, { title: '过期时间', dataIndex: 'expires_at' }]} /> : <Empty description="暂无第三方注册终端" />}</div>;
}

export function CallerPolicyForm({ policy, set, pools, numbers }: { policy: OutboundPolicy; set: (key: keyof OutboundPolicy, value: unknown) => void; pools: Entity[]; numbers: Entity[] }) {
  const numberOptions = numbers.map((item) => ({ label: String(item.number), value: String(item.number) }));
  const poolOptions = pools.map((item) => ({ label: String(item.virtual_alias || item.id), value: String(item.id) }));
  return <Form layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}>
    <Field label="主叫策略" required><Select value={policy.caller_mode} options={callerOptions} onChange={(value) => set('caller_mode', value)} /></Field>
    <Field label="失败处理" required><Select value={policy.fallback_mode} options={[{ label: '拒绝呼叫', value: 'reject' }, { label: '固定替换', value: 'fixed' }, { label: '号码池替换', value: 'pool' }]} onChange={(value) => set('fallback_mode', value)} /></Field>
    {policy.caller_mode === 'fixed_number' && <Field label="固定号码" required><Select value={policy.fixed_number} options={numberOptions} onChange={(value) => set('fixed_number', value)} placeholder="选择已授权真实号码" /></Field>}
    {policy.caller_mode === 'virtual_pool' && <Field label="主叫号码池" required><Select value={policy.caller_pool_id} options={poolOptions} onChange={(value) => set('caller_pool_id', value)} placeholder="选择当前来源的号码池" /></Field>}
    {policy.fallback_mode !== 'reject' && <Field label="失败替换"><Alert type="info" content="替换号码或备用池将在号码池成员配置中指定，并记录到 CDR。" /></Field>}
  </Grid.Row></Form>;
}

export function EgressBindingForm({ policy, set, groups, trunks }: { policy: OutboundPolicy; set: (key: keyof OutboundPolicy, value: unknown) => void; groups: Entity[]; trunks: Entity[] }) {
  const options = (items: Entity[]) => items.map((item) => ({ label: String(item.name ?? item.id), value: String(item.id) }));
  return <Form layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}><Field label="绑定方式" required><Select value={policy.egress_mode} options={[{ label: '直接中继', value: 'direct' }, { label: '落地分组', value: 'group' }]} onChange={(value) => set('egress_mode', value)} /></Field>{policy.egress_mode === 'direct' ? <Field label="落地中继" required><Select value={policy.direct_egress_trunk_id} options={options(trunks.filter((item) => trunkRole(item) === 'egress'))} onChange={(value) => set('direct_egress_trunk_id', value)} placeholder="选择唯一号码归属中继" /></Field> : <Field label="落地分组" required><Select value={policy.egress_group_id} options={options(groups)} onChange={(value) => set('egress_group_id', value)} placeholder="选择允许使用的落地范围" /></Field>}</Grid.Row></Form>;
}

export default function TrunkDetailPage() {
  const { id = '' } = useParams();
  const [data, setData] = useState<TrunkWorkspaceData | null>(null);
  const [draft, setDraft] = useState<Entity>({});
  const [rules, setRules] = useState<TrunkIpRule[]>([]);
  const [endpoints, setEndpoints] = useState<EgressEndpoint[]>([]);
  const [policy, setPolicy] = useState<OutboundPolicy>(emptyPolicy);
  const [groups, setGroups] = useState<Entity[]>([]);
  const [pools, setPools] = useState<Entity[]>([]);
  const [trunks, setTrunks] = useState<Entity[]>([]);
  const [loading, setLoading] = useState(true); const [saving, setSaving] = useState(false); const [error, setError] = useState('');
  const role = trunkRole(draft);
  const load = useCallback(async () => {
    setLoading(true); setError('');
    try {
      const workspace = await getTrunkWorkspace(id); setData(workspace); setDraft({ ...workspace.trunk, role: trunkRole(workspace.trunk) });
      const optional = await Promise.allSettled([
        getTrunkIpRules(id),
        getOutboundPolicy(id),
        listOptions('/egress-groups'),
        listOptions('/trunks'),
        listOptions('/caller-pools'),
        getTrunkEgressEndpoints(id),
      ]);
      if (optional[0].status === 'fulfilled') setRules(optional[0].value.map((rule) => ({ ...rule, _key: rule.id || genId() })));
      if (optional[1].status === 'fulfilled') setPolicy({ ...emptyPolicy, ...optional[1].value });
      if (optional[2].status === 'fulfilled') setGroups(optional[2].value);
      if (optional[3].status === 'fulfilled') setTrunks(optional[3].value);
      if (optional[4].status === 'fulfilled') setPools(optional[4].value.filter((pool) => pool.owner_source_type === 'trunk' && pool.owner_source_id === id));
      if (optional[5].status === 'fulfilled') setEndpoints(optional[5].value.map((ep) => ({ ...ep, _key: ep.id || genId() })));
    } catch (reason) { setError(reason instanceof Error ? reason.message : '中继加载失败'); }
    finally { setLoading(false); }
  }, [id]);
  useEffect(() => { void load(); }, [load]);
  const set = (key: string, value: unknown) => setDraft((current) => ({ ...current, [key]: value }));
  const setPolicyField = (key: keyof OutboundPolicy, value: unknown) => setPolicy((current) => ({ ...current, [key]: value }));
  const save = async () => {
    if (role === 'access' && ['ip_allowlist', 'ip_and_digest'].includes(String(draft.access_auth_mode)) && (!rules.length || rules.some((rule) => !rule.cidr.trim()))) { Message.error('请至少配置一条完整的 IP 白名单'); return; }
    if (role === 'egress' && endpoints.some((ep) => !ep.host.trim())) { Message.error('落地端点的主机地址不能为空'); return; }
    const policyError = role === 'access' ? policyValidationError(policy) : null;
    if (policyError) { Message.error(policyError); return; }
    try {
      setSaving(true);
      const body = { ...draft, supports_registration: ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode)), reg_auth_type: String(draft.access_auth_mode).includes('digest') ? 'digest' : 'ip' };
      await updateTrunk(id, body);
      if (role === 'access') {
        await Promise.all([saveTrunkIpRules(id, rules), saveOutboundPolicy(id, policy)]);
      } else if (role === 'egress') {
        await saveTrunkEgressEndpoints(id, endpoints);
      }
      Message.success('中继配置已保存');
      await load();
    }
    catch (reason) { Message.error(reason instanceof Error ? reason.message : '保存失败'); }
    finally { setSaving(false); }
  };
  const tabs = useMemo(() => {
    const list = [
      { key: 'basic', title: '基本配置', content: <BasicTab draft={draft} set={set} /> },
      { key: 'auth', title: '接入认证', content: role === 'access' ? <AccessAuthTab draft={draft} set={set} rules={rules} setRules={setRules} /> : null, hide: role !== 'access' },
      { key: 'registration', title: '注册状态', content: role === 'egress' ? <RegistrationTab draft={draft} set={set} registrations={data?.registrations || []} endpoints={endpoints} setEndpoints={setEndpoints} /> : <AccessRegistrationStatus registrations={data?.registrations || []} /> },
      { key: 'caller', title: '主叫策略', content: role === 'access' ? <CallerPolicyForm policy={policy} set={setPolicyField} pools={pools} numbers={data?.numbers || []} /> : null, hide: role !== 'access' },
      { key: 'pool', title: role === 'access' ? '号码池组' : '归属号码', content: role === 'access' ? <div className="section-block"><div className="section-title"><h2>号码池组</h2><Tag>{policy.caller_mode === 'virtual_pool' ? policy.caller_pool_id || '尚未绑定' : '当前策略不使用号码池'}</Tag></div><p className="muted-copy">号码池成员是唯一归属于落地中继的真实号码。请在“号码池组”页面维护成员和选号算法。</p></div> : <div className="section-block"><h2>归属号码</h2>{data?.numbers?.length ? <Table pagination={false} data={data.numbers} columns={[{ title: '真实号码', dataIndex: 'number' }, { title: '可做主叫', dataIndex: 'can_present' }, { title: '可接呼入', dataIndex: 'can_receive' }]} /> : <Empty description="该落地中继暂无归属号码" />}</div> },
      { key: 'binding', title: '落地绑定', content: role === 'access' ? <EgressBindingForm policy={policy} set={setPolicyField} groups={groups} trunks={trunks} /> : null, hide: role !== 'access' },
    ];
    return list.filter((tab) => !tab.hide);
  }, [data, draft, groups, policy, pools, role, rules, trunks, endpoints]);
  if (loading) {
    return <section className="workspace"><Header title="加载中..." loading={true} saving={saving} onRefresh={load} onSave={save} /><div style={{ minHeight: 300, display: 'flex', alignItems: 'center', justifyContent: 'center' }}><Spin loading={true} size={36} /></div></section>;
  }
  return <section className="workspace"><Header title={String(draft.id || '中继详情')} loading={loading} saving={saving} onRefresh={load} onSave={save} />{error ? <Alert type="error" title="数据加载失败" content={error} /> : <div className="trunk-workspace"><Tabs defaultActiveTab="basic">{tabs.map((tab) => <Tabs.TabPane key={tab.key} title={tab.title}>{tab.content}</Tabs.TabPane>)}</Tabs></div>}</section>;
}
