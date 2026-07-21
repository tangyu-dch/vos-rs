import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Button, Card, CardBody, Chip, Input,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Tabs, Tab,
} from '@heroui/react';
import { RefreshCw, Phone, ShieldCheck } from 'lucide-react';
import { useParams } from 'react-router-dom';
import type { Entity } from '@/services/resources';
import { getExtensionOutboundPolicy, getExtensionWorkspace, saveExtensionOutboundPolicy, updateExtensionPassword, type ExtensionWorkspaceData } from '@/services/extensions';
import { listOptions, policyValidationError, type OutboundPolicy } from '@/services/trunks';
import { CallerPolicyForm, EgressBindingForm, WorkspaceField, emptyPolicy } from '@/pages/trunks/trunk-detail';
import { DetailErrorState, DetailHeader, DetailLoading, FormGrid, SectionBlock } from '@/components/detail-shell';
import { message } from '@/utils/toast';

function RegistrationStatus({
  registrations,
  onRefresh,
  refreshing,
}: {
  registrations: Entity[];
  onRefresh?: () => void;
  refreshing?: boolean;
}) {
  const isOnline = registrations.length > 0;
  return (
    <SectionBlock
      title="注册终端"
      description="分机可同时注册多个终端，外呼策略始终按认证分机识别。"
      actions={
        <div className="flex items-center gap-2">
          <Chip
            size="sm"
            variant="flat"
            color={isOnline ? 'success' : 'default'}
            startContent={
              <span className={`w-2 h-2 rounded-full ${isOnline ? 'bg-success animate-pulse' : 'bg-default-400'}`} />
            }
          >
            {isOnline ? `${registrations.length} 个在线终端` : '当前离线'}
          </Chip>
          {onRefresh && (
            <Button
              size="sm"
              variant="flat"
              isLoading={refreshing}
              onPress={onRefresh}
              startContent={<RefreshCw className="w-3.5 h-3.5" />}
            >
              刷新在线状态
            </Button>
          )}
        </div>
      }
    >
      {registrations.length ? (
        <Table aria-label="注册终端列表">
          <TableHeader>
            <TableColumn key="contact">联系地址</TableColumn>
            <TableColumn key="received_from">来源 Socket / 设备标识</TableColumn>
            <TableColumn key="status">状态</TableColumn>
            <TableColumn key="expires_at">过期时间</TableColumn>
          </TableHeader>
          <TableBody items={registrations}>
            {(row) => (
              <TableRow key={String(row.contact_uri ?? row.contact ?? row.id ?? row.user_agent ?? '')}>
                <TableCell className="font-mono text-tiny">{String(row.contact_uri ?? row.contact ?? '')}</TableCell>
                <TableCell>{String(row.received_from ?? row.user_agent ?? row.node ?? '')}</TableCell>
                <TableCell>
                  <Chip size="sm" variant="dot" color="success">在线</Chip>
                </TableCell>
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

  const isOnline = Boolean(data?.registrations && data.registrations.length > 0);
  const regCount = data?.registrations?.length ?? 0;

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
    { key: 'registration', title: '注册状态', content: <RegistrationStatus registrations={data?.registrations || []} onRefresh={load} refreshing={loading} /> },
    { key: 'caller', title: '主叫策略', content: <CallerPolicyForm policy={policy} set={setPolicyField} pools={pools} numbers={data?.numbers || []} /> },
    { key: 'binding', title: '落地绑定', content: <EgressBindingForm policy={policy} set={setPolicyField} groups={groups} trunks={trunks} /> },
    { key: 'numbers', title: '号码归属', content: <NumberOwnership numbers={data?.numbers || []} /> },
  ], [confirmPassword, data, groups, loading, load, password, policy, pools, trunks, username]);

  if (loading) {
    return (
      <section>
        <DetailHeader loading={true} saving={saving} onRefresh={load} onSave={save} />
        <DetailLoading />
      </section>
    );
  }

  return (
    <section className="flex flex-col gap-4">
      {/* 顶部分机状态概览栏 */}
      <div className="flex flex-wrap items-center justify-between gap-4 p-4 bg-content1 rounded-xl border border-default-200">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center text-primary">
            <Phone className="w-5 h-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-bold text-foreground">分机 {username}</h2>
              <Chip
                size="sm"
                variant="flat"
                color={isOnline ? 'success' : 'default'}
                startContent={<span className={`w-2 h-2 rounded-full ${isOnline ? 'bg-success animate-pulse' : 'bg-default-400'}`} />}
              >
                {isOnline ? `在线 (${regCount} 个终端)` : '未注册/离线'}
              </Chip>
              {Boolean(data?.credential?.configured) && (
                <Chip size="sm" variant="dot" color="primary" startContent={<ShieldCheck className="w-3 h-3" />}>
                  Digest 凭据就绪
                </Chip>
              )}
            </div>
            <p className="text-tiny text-default-400 mt-0.5">SIP 账号管理、注册终端追踪与出站路由策略配置</p>
          </div>
        </div>

        <DetailHeader loading={loading} saving={saving} onRefresh={load} onSave={save} />
      </div>

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
