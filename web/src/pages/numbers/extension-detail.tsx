import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Card, CardBody, Chip, Input,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Tabs, Tab,
} from '@heroui/react';
import { useParams } from 'react-router-dom';
import type { Entity } from '@/services/resources';
import { getExtensionOutboundPolicy, getExtensionWorkspace, saveExtensionOutboundPolicy, updateExtensionPassword, type ExtensionWorkspaceData } from '@/services/extensions';
import { listOptions, policyValidationError, type OutboundPolicy } from '@/services/trunks';
import { CallerPolicyForm, EgressBindingForm, WorkspaceField, emptyPolicy } from '@/pages/trunks/trunk-detail';
import { DetailErrorState, DetailHeader, DetailLoading, FormGrid, SectionBlock } from '@/components/detail-shell';
import { message } from '@/utils/toast';

function RegistrationStatus({ registrations }: { registrations: Entity[] }) {
  return (
    <SectionBlock
      title="注册终端"
      description="分机可同时注册多个终端，外呼策略始终按认证分机识别。"
      actions={
        <Chip size="sm" variant="flat" color={registrations.length ? 'success' : 'default'}>
          {registrations.length ? `${registrations.length} 个在线终端` : '当前离线'}
        </Chip>
      }
    >
      {registrations.length ? (
        <Table aria-label="注册终端列表">
          <TableHeader>
            <TableColumn key="contact">联系地址</TableColumn>
            <TableColumn key="received_from">来源 Socket / 设备标识</TableColumn>
            <TableColumn key="expires_at">过期时间</TableColumn>
          </TableHeader>
          <TableBody items={registrations}>
            {(row) => (
              <TableRow key={String(row.contact_uri ?? row.contact ?? row.id ?? row.user_agent ?? '')}>
                <TableCell>{String(row.contact_uri ?? row.contact ?? '')}</TableCell>
                <TableCell>{String(row.received_from ?? row.user_agent ?? row.node ?? '')}</TableCell>
                <TableCell>{String(row.expires_at ?? '')}</TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      ) : (
        <p className="text-tiny text-default-400">暂无在线注册终端</p>
      )}
    </SectionBlock>
  );
}

function NumberOwnership({ numbers }: { numbers: Entity[] }) {
  return (
    <SectionBlock
      title="号码归属"
      description="分机使用授权与号码物理落地归属相互独立。"
      actions={<Chip size="sm" variant="flat">{numbers.length} 个号码</Chip>}
    >
      {numbers.length ? (
        <Table aria-label="号码归属列表">
          <TableHeader>
            <TableColumn key="number">真实号码</TableColumn>
            <TableColumn key="owner_egress_trunk_id">落地中继</TableColumn>
            <TableColumn key="can_receive">允许呼入</TableColumn>
            <TableColumn key="can_present">允许显号</TableColumn>
          </TableHeader>
          <TableBody items={numbers}>
            {(row) => {
              const canReceive = row.can_receive ?? ['inbound', 'both', 'bidirectional'].includes(String(row.direction));
              const canPresent = row.can_present ?? ['outbound', 'both', 'bidirectional'].includes(String(row.direction));
              return (
                <TableRow key={String(row.number ?? row.id ?? row.owner_egress_trunk_id ?? '')}>
                  <TableCell>{String(row.number ?? '')}</TableCell>
                  <TableCell>{String(row.owner_egress_trunk_id ?? '')}</TableCell>
                  <TableCell>{canReceive ? '是' : '否'}</TableCell>
                  <TableCell>{canPresent ? '是' : '否'}</TableCell>
                </TableRow>
              );
            }}
          </TableBody>
        </Table>
      ) : (
        <p className="text-tiny text-default-400">该分机尚未获授权真实号码</p>
      )}
    </SectionBlock>
  );
}

interface TabDef {
  key: string;
  title: string;
  content: ReactNode;
}

export default function ExtensionDetailPage() {
  const { id: username = '' } = useParams();
  const [data, setData] = useState<ExtensionWorkspaceData | null>(null);
  const [policy, setPolicy] = useState<OutboundPolicy>(emptyPolicy);
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [groups, setGroups] = useState<Entity[]>([]);
  const [trunks, setTrunks] = useState<Entity[]>([]);
  const [pools, setPools] = useState<Entity[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const workspace = await getExtensionWorkspace(username);
      setData(workspace);
      const optional = await Promise.allSettled([
        getExtensionOutboundPolicy(username),
        listOptions('/egress-groups'),
        listOptions('/trunks'),
        listOptions('/caller-pools'),
      ]);
      if (optional[0].status === 'fulfilled') setPolicy({ ...emptyPolicy, ...optional[0].value });
      if (optional[1].status === 'fulfilled') setGroups(optional[1].value);
      if (optional[2].status === 'fulfilled') setTrunks(optional[2].value);
      if (optional[3].status === 'fulfilled') setPools(optional[3].value.filter((pool) => pool.owner_source_type === 'extension' && pool.owner_source_id === username));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '分机加载失败');
    } finally {
      setLoading(false);
    }
  }, [username]);

  useEffect(() => { void load(); }, [load]);

  const setPolicyField = (key: keyof OutboundPolicy, value: unknown) => setPolicy((current) => ({ ...current, [key]: value }));

  const save = async () => {
    const policyError = policyValidationError(policy);
    if (policyError) { message.error(policyError); return; }
    if (password !== confirmPassword) { message.error('两次输入的新密码不一致'); return; }
    try {
      setSaving(true);
      if (password) await updateExtensionPassword(username, password);
      await saveExtensionOutboundPolicy(username, policy);
      setPassword('');
      setConfirmPassword('');
      message.success('分机配置已保存');
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '保存失败');
    } finally {
      setSaving(false);
    }
  };

  const tabs = useMemo<TabDef[]>(() => [
    {
      key: 'basic',
      title: '基本配置',
      content: (
        <FormGrid>
          <WorkspaceField label="分机号码" required>
            <Input variant="bordered" value={username} isDisabled />
          </WorkspaceField>
          <WorkspaceField label="凭据状态">
            <Input
              variant="bordered"
              isDisabled
              value={data?.credential?.configured ? '已配置 Digest 凭据' : '尚未配置凭据'}
            />
          </WorkspaceField>
          <WorkspaceField label="更新密码">
            <Input
              type="password"
              variant="bordered"
              value={password}
              onValueChange={setPassword}
              placeholder="留空表示不修改"
            />
          </WorkspaceField>
          <WorkspaceField label="确认密码">
            <Input
              type="password"
              variant="bordered"
              value={confirmPassword}
              onValueChange={setConfirmPassword}
              placeholder="再次输入新密码"
            />
          </WorkspaceField>
        </FormGrid>
      ),
    },
    { key: 'registration', title: '注册状态', content: <RegistrationStatus registrations={data?.registrations || []} /> },
    { key: 'caller', title: '主叫策略', content: <CallerPolicyForm policy={policy} set={setPolicyField} pools={pools} numbers={data?.numbers || []} /> },
    { key: 'binding', title: '落地绑定', content: <EgressBindingForm policy={policy} set={setPolicyField} groups={groups} trunks={trunks} /> },
    { key: 'numbers', title: '号码归属', content: <NumberOwnership numbers={data?.numbers || []} /> },
  ], [confirmPassword, data, groups, password, policy, pools, trunks, username]);

  if (loading) {
    return (
      <section>
        <DetailHeader loading={true} saving={saving} onRefresh={load} onSave={save} />
        <DetailLoading />
      </section>
    );
  }

  return (
    <section>
      <DetailHeader loading={loading} saving={saving} onRefresh={load} onSave={save} />
      {error ? (
        <DetailErrorState error={error} />
      ) : (
        <Card>
          <CardBody className="p-6">
            <Tabs aria-label="分机配置">
              {tabs.map((tab) => (
                <Tab key={tab.key} title={tab.title}>
                  {tab.content}
                </Tab>
              ))}
            </Tabs>
          </CardBody>
        </Card>
      )}
    </section>
  );
}
