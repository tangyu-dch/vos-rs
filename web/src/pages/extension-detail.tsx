import { useCallback, useEffect, useMemo, useState } from 'react';
import { Alert, Button, Empty, Form, Grid, Input, Message, Space, Spin, Table, Tabs, Tag } from '@arco-design/web-react';
import { IconRefresh, IconSave } from '@arco-design/web-react/icon';
import { useParams } from 'react-router-dom';
import type { Entity } from '../services/resources';
import { getExtensionOutboundPolicy, getExtensionWorkspace, saveExtensionOutboundPolicy, updateExtensionPassword, type ExtensionWorkspaceData } from '../services/extensions';
import { listOptions, policyValidationError, type OutboundPolicy } from '../services/trunks';
import { CallerPolicyForm, EgressBindingForm, WorkspaceField, emptyPolicy } from './trunk-detail';

function RegistrationStatus({ registrations }: { registrations: Entity[] }) {
  return <div className="section-block"><div className="section-title"><div><h2>注册终端</h2><p className="muted-copy">分机可同时注册多个终端，外呼策略始终按认证分机识别。</p></div><Tag color={registrations.length ? 'green' : 'gray'}>{registrations.length ? `${registrations.length} 个在线终端` : '当前离线'}</Tag></div>{registrations.length ? <Table pagination={false} data={registrations} columns={[{ title: '联系地址', dataIndex: 'contact' }, { title: '设备标识', dataIndex: 'user_agent' }, { title: '过期时间', dataIndex: 'expires_at' }]} /> : <Empty description="暂无在线注册终端" />}</div>;
}

function NumberOwnership({ numbers }: { numbers: Entity[] }) {
  return <div className="section-block"><div className="section-title"><div><h2>号码归属</h2><p className="muted-copy">分机使用授权与号码物理落地归属相互独立。</p></div><Tag>{numbers.length} 个号码</Tag></div>{numbers.length ? <Table pagination={false} data={numbers} columns={[{ title: '真实号码', dataIndex: 'number' }, { title: '落地中继', dataIndex: 'owner_egress_trunk_id' }, { title: '允许呼入', dataIndex: 'can_receive', render: (value, row) => value ?? ['inbound', 'both', 'bidirectional'].includes(String(row.direction)) ? '是' : '否' }, { title: '允许显号', dataIndex: 'can_present', render: (value, row) => value ?? ['outbound', 'both', 'bidirectional'].includes(String(row.direction)) ? '是' : '否' }]} /> : <Empty description="该分机尚未获授权真实号码" />}</div>;
}

export default function ExtensionDetailPage() {
  const { id: username = '' } = useParams();
  const [data, setData] = useState<ExtensionWorkspaceData | null>(null);
  const [policy, setPolicy] = useState<OutboundPolicy>(emptyPolicy);
  const [password, setPassword] = useState(''); const [confirmPassword, setConfirmPassword] = useState('');
  const [groups, setGroups] = useState<Entity[]>([]); const [trunks, setTrunks] = useState<Entity[]>([]); const [pools, setPools] = useState<Entity[]>([]);
  const [loading, setLoading] = useState(true); const [saving, setSaving] = useState(false); const [error, setError] = useState('');
  const load = useCallback(async () => {
    setLoading(true); setError('');
    try {
      const workspace = await getExtensionWorkspace(username); setData(workspace);
      const optional = await Promise.allSettled([getExtensionOutboundPolicy(username), listOptions('/egress-groups'), listOptions('/trunks'), listOptions('/caller-pools')]);
      if (optional[0].status === 'fulfilled') setPolicy({ ...emptyPolicy, ...optional[0].value });
      if (optional[1].status === 'fulfilled') setGroups(optional[1].value);
      if (optional[2].status === 'fulfilled') setTrunks(optional[2].value);
      if (optional[3].status === 'fulfilled') setPools(optional[3].value.filter((pool) => pool.owner_source_type === 'extension' && pool.owner_source_id === username));
    } catch (reason) { setError(reason instanceof Error ? reason.message : '分机加载失败'); }
    finally { setLoading(false); }
  }, [username]);
  useEffect(() => { void load(); }, [load]);
  const setPolicyField = (key: keyof OutboundPolicy, value: unknown) => setPolicy((current) => ({ ...current, [key]: value }));
  const save = async () => {
    const policyError = policyValidationError(policy); if (policyError) { Message.error(policyError); return; }
    if (password !== confirmPassword) { Message.error('两次输入的新密码不一致'); return; }
    try { setSaving(true); if (password) await updateExtensionPassword(username, password); await saveExtensionOutboundPolicy(username, policy); setPassword(''); setConfirmPassword(''); Message.success('分机配置已保存'); await load(); }
    catch (reason) { Message.error(reason instanceof Error ? reason.message : '保存失败'); }
    finally { setSaving(false); }
  };
  const tabs = useMemo(() => [
    { key: 'basic', title: '基本配置', content: <Form layout="vertical"><Grid.Row className="form-grid" gutter={[18, 0]}><WorkspaceField label="分机号码" required><Input value={username} disabled /></WorkspaceField><WorkspaceField label="凭据状态"><Input value={data?.credential?.configured ? '已配置 Digest 凭据' : '尚未配置凭据'} disabled /></WorkspaceField><WorkspaceField label="更新密码"><Input.Password value={password} onChange={setPassword} placeholder="留空表示不修改" /></WorkspaceField><WorkspaceField label="确认密码"><Input.Password value={confirmPassword} onChange={setConfirmPassword} placeholder="再次输入新密码" /></WorkspaceField></Grid.Row></Form> },
    { key: 'registration', title: '注册状态', content: <RegistrationStatus registrations={data?.registrations || []} /> },
    { key: 'caller', title: '主叫策略', content: <CallerPolicyForm policy={policy} set={setPolicyField} pools={pools} numbers={data?.numbers || []} /> },
    { key: 'binding', title: '落地绑定', content: <EgressBindingForm policy={policy} set={setPolicyField} groups={groups} trunks={trunks} /> },
    { key: 'numbers', title: '号码归属', content: <NumberOwnership numbers={data?.numbers || []} /> },
  ], [confirmPassword, data, groups, password, policy, pools, trunks, username]);
  return <section className="workspace"><header className="page-header"><div><h1>{username || '分机详情'}</h1><p>分机注册、主叫号码与公网落地策略。</p></div><Space><Button icon={<IconRefresh />} loading={loading} onClick={load}>刷新</Button><Button type="primary" icon={<IconSave />} loading={saving} onClick={save}>保存配置</Button></Space></header>{error ? <Alert type="error" title="数据加载失败" content={error} /> : <Spin loading={loading} block><div className="trunk-workspace"><Tabs defaultActiveTab="basic">{tabs.map((tab) => <Tabs.TabPane key={tab.key} title={tab.title}>{tab.content}</Tabs.TabPane>)}</Tabs></div></Spin>}</section>;
}
