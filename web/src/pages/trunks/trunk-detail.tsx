import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Button, Card, CardBody, Chip, Input, Select, SelectItem, Switch,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Tabs, Tab,
} from '@heroui/react';
import { Plus, Trash2, RefreshCw, Server } from 'lucide-react';
import { useParams } from 'react-router-dom';
import type { Entity } from '@/services/resources';
import {
  getOutboundPolicy, getTrunkIpRules, getTrunkWorkspace, listOptions, saveOutboundPolicy,
  policyValidationError, saveTrunkIpRules, trunkRole, trunkValidationError, updateTrunk, type OutboundPolicy, type TrunkIpRule,
  type TrunkWorkspaceData, getTrunkEgressEndpoints, saveTrunkEgressEndpoints, type EgressEndpoint,
} from '@/services/trunks';
import { DetailErrorState, DetailHeader, DetailLoading, FormGrid, SectionBlock } from '@/components/detail-shell';
import { message } from '@/utils/toast';

export const emptyPolicy: OutboundPolicy = {
  caller_mode: 'strict_passthrough', fallback_mode: 'reject',
  egress_mode: 'direct', enabled: true,
};

const roleOptions = [
  { label: '接入中继', value: 'access' },
  { label: '落地中继', value: 'egress' },
];
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
const egressModeOptions = [
  { label: '直接中继', value: 'direct' },
  { label: '落地分组', value: 'group' },
];
const connectionTypeOptions = [
  { label: 'IP 直连', value: 'static_peer' },
  { label: '主动注册', value: 'client_register' },
];

const genId = () => (crypto.randomUUID ? crypto.randomUUID() : Math.random().toString(36).substring(2));

interface FieldProps {
  label: string;
  children: ReactNode;
  fullWidth?: boolean;
  required?: boolean;
}

/** 表单字段容器：标签 + 控件，配合 grid 父容器使用 */
export function WorkspaceField({ label, children, fullWidth = false, required = false }: FieldProps) {
  return (
    <div className={fullWidth ? 'md:col-span-2 col-span-1' : 'col-span-1'}>
      <label className="block text-tiny font-medium text-foreground mb-1.5">
        {label}
        {required && <span className="text-danger ml-0.5">*</span>}
      </label>
      {children}
    </div>
  );
}
const Field = WorkspaceField;

function BasicTab({ draft, set }: { draft: Entity; set: (key: string, value: unknown) => void }) {
  const role = trunkRole(draft);
  return (
    <FormGrid>
      <Field label="中继标识" required>
        <Input variant="bordered" value={String(draft.id ?? '')} isDisabled />
      </Field>
      <Field label="中继类型" required>
        <Select variant="bordered" selectedKeys={[role]} isDisabled>
          {roleOptions.map((opt) => (
            <SelectItem key={opt.value}>{opt.label}</SelectItem>
          ))}
        </Select>
      </Field>
      <Field label="计费账户">
        <Input
          type="number"
          variant="bordered"
          value={draft.account_id !== undefined && draft.account_id !== null ? String(draft.account_id) : ''}
          onValueChange={(v) => set('account_id', v === '' ? null : Number(v))}
          placeholder="可选"
        />
      </Field>
      {role === 'egress' && (
        <>
          <Field label="主机地址" required>
            <Input variant="bordered" value={String(draft.host ?? '')} onValueChange={(v) => set('host', v)} />
          </Field>
          <Field label="SIP 端口" required>
            <Input
              type="number"
              variant="bordered"
              value={String(draft.port ?? 5060)}
              onValueChange={(v) => set('port', Number(v) || 5060)}
              min={1}
              max={65535}
            />
          </Field>
        </>
      )}
      <Field label="容量上限">
        <Input
          type="number"
          variant="bordered"
          value={String(draft.max_capacity ?? 100)}
          onValueChange={(v) => set('max_capacity', Number(v) || 0)}
          min={0}
        />
      </Field>
      <Field label="启用状态">
        <Switch
          isSelected={draft.enabled !== false}
          onChange={(e) => set('enabled', e.target.checked)}
        />
      </Field>
    </FormGrid>
  );
}

function IpRulesEditor({ rules, onChange }: { rules: TrunkIpRule[]; onChange: (rules: TrunkIpRule[]) => void }) {
  const patch = (index: number, values: Partial<TrunkIpRule>) =>
    onChange(rules.map((rule, itemIndex) => (itemIndex === index ? { ...rule, ...values } : rule)));

  return (
    <SectionBlock
      title="来源地址"
      description="支持 IPv4、IPv6 和 CIDR；端口留空表示任意来源端口。"
      actions={
        <Button
          size="sm"
          variant="flat"
          onPress={() => onChange([...rules, { _key: genId(), cidr: '', source_port: null, transport: 'udp', description: '', enabled: true }])}
          startContent={<Plus className="w-4 h-4" />}
        >
          添加地址
        </Button>
      }
    >
      <Table aria-label="IP 白名单规则" isStriped>
        <TableHeader>
          <TableColumn key="cidr">来源 IP/CIDR</TableColumn>
          <TableColumn key="source_port">来源端口</TableColumn>
          <TableColumn key="transport">传输协议</TableColumn>
          <TableColumn key="description">备注说明</TableColumn>
          <TableColumn key="enabled">启用</TableColumn>
          <TableColumn key="actions" align="end">操作</TableColumn>
        </TableHeader>
        <TableBody items={rules} emptyContent="尚未配置 IP 白名单">
          {(row) => {
            const idx = rules.findIndex((r) => (r._key || r.id || r.cidr) === (row._key || row.id || row.cidr));
            const rowKey = row._key || row.id || row.cidr || idx;
            return (
              <TableRow key={rowKey}>
                <TableCell>
                  <Input
                    variant="underlined"
                    value={row.cidr}
                    onValueChange={(v) => patch(idx, { cidr: v })}
                    placeholder="例如 192.0.2.10/32"
                  />
                </TableCell>
                <TableCell>
                  <Input
                    variant="underlined"
                    type="number"
                    value={row.source_port !== null && row.source_port !== undefined ? String(row.source_port) : ''}
                    onValueChange={(v) => patch(idx, { source_port: v === '' ? null : Number(v) || null })}
                    placeholder="任意"
                    min={1}
                    max={65535}
                  />
                </TableCell>
                <TableCell>
                  <Select variant="underlined" selectedKeys={['udp']} isDisabled>
                    <SelectItem key="udp">UDP</SelectItem>
                  </Select>
                </TableCell>
                <TableCell>
                  <Input
                    variant="underlined"
                    value={row.description}
                    onValueChange={(v) => patch(idx, { description: v })}
                  />
                </TableCell>
                <TableCell>
                  <Switch
                    size="sm"
                    isSelected={row.enabled}
                    onChange={(e) => patch(idx, { enabled: e.target.checked })}
                  />
                </TableCell>
                <TableCell>
                  <Button
                    isIconOnly
                    size="sm"
                    color="danger"
                    variant="light"
                    onPress={() => onChange(rules.filter((_, itemIndex) => itemIndex !== idx))}
                  >
                    <Trash2 className="w-4 h-4 text-danger" />
                  </Button>
                </TableCell>
              </TableRow>
            );
          }}
        </TableBody>
      </Table>
    </SectionBlock>
  );
}

function EgressEndpointsEditor({ endpoints, onChange }: { endpoints: EgressEndpoint[]; onChange: (endpoints: EgressEndpoint[]) => void }) {
  const patch = (index: number, values: Partial<EgressEndpoint>) =>
    onChange(endpoints.map((ep, itemIndex) => (itemIndex === index ? { ...ep, ...values } : ep)));

  return (
    <SectionBlock
      title="落地端点 (Egress Endpoints)"
      description="静态直连中继可以配置多个落地服务器端点，支持优先级故障切换。"
      actions={
        <Button
          size="sm"
          variant="flat"
          onPress={() => onChange([...endpoints, { _key: genId(), host: '', port: 5060, transport: 'udp', priority: 100, enabled: true }])}
          startContent={<Plus className="w-4 h-4" />}
        >
          添加端点
        </Button>
      }
    >
      <Table aria-label="落地端点列表" isStriped>
        <TableHeader>
          <TableColumn key="host">落地主机/IP</TableColumn>
          <TableColumn key="port">SIP 端口</TableColumn>
          <TableColumn key="transport">传输协议</TableColumn>
          <TableColumn key="priority">优先级</TableColumn>
          <TableColumn key="enabled">启用</TableColumn>
          <TableColumn key="actions" align="end">操作</TableColumn>
        </TableHeader>
        <TableBody items={endpoints} emptyContent="尚未配置落地端点">
          {(row) => {
            const idx = endpoints.findIndex((e) => (e._key || e.id || e.host) === (row._key || row.id || row.host));
            const rowKey = row._key || row.id || row.host || idx;
            return (
              <TableRow key={rowKey}>
                <TableCell>
                  <Input
                    variant="underlined"
                    value={row.host}
                    onValueChange={(v) => patch(idx, { host: v })}
                    placeholder="例如 203.0.113.50 或 sip.carrier.com"
                  />
                </TableCell>
                <TableCell>
                  <Input
                    variant="underlined"
                    type="number"
                    value={row.port !== null && row.port !== undefined ? String(row.port) : ''}
                    onValueChange={(v) => patch(idx, { port: v === '' ? null : Number(v) || null })}
                    placeholder="5060"
                    min={1}
                    max={65535}
                  />
                </TableCell>
                <TableCell>
                  <Select variant="underlined" selectedKeys={['udp']} isDisabled>
                    <SelectItem key="udp">UDP</SelectItem>
                  </Select>
                </TableCell>
                <TableCell>
                  <Input
                    variant="underlined"
                    type="number"
                    value={String(row.priority ?? 100)}
                    onValueChange={(v) => patch(idx, { priority: Number(v) || 100 })}
                    min={0}
                    max={65535}
                  />
                </TableCell>
                <TableCell>
                  <Switch
                    size="sm"
                    isSelected={row.enabled}
                    onChange={(e) => patch(idx, { enabled: e.target.checked })}
                  />
                </TableCell>
                <TableCell>
                  <Button
                    isIconOnly
                    size="sm"
                    color="danger"
                    variant="light"
                    onPress={() => onChange(endpoints.filter((_, itemIndex) => itemIndex !== idx))}
                  >
                    <Trash2 className="w-4 h-4 text-danger" />
                  </Button>
                </TableCell>
              </TableRow>
            );
          }}
        </TableBody>
      </Table>
    </SectionBlock>
  );
}

function AccessAuthTab({ draft, set, rules, setRules }: { draft: Entity; set: (key: string, value: unknown) => void; rules: TrunkIpRule[]; setRules: (rules: TrunkIpRule[]) => void }) {
  const mode = String(draft.access_auth_mode ?? 'ip_allowlist');
  const showIp = mode === 'ip_allowlist' || mode === 'ip_and_digest';
  const showDigest = mode === 'digest_register' || mode === 'ip_and_digest';
  return (
    <div className="flex flex-col gap-5">
      <FormGrid>
        <Field label="认证方式" required>
          <Select
            variant="bordered"
            selectedKeys={[mode]}
            onChange={(e) => set('access_auth_mode', e.target.value)}
          >
            {authOptions.map((opt) => (
              <SelectItem key={opt.value}>{opt.label}</SelectItem>
            ))}
          </Select>
        </Field>
        {showDigest && (
          <>
            <Field label="认证 Realm" required>
              <Input variant="bordered" isDisabled value={String(draft.access_realm ?? 'vos-rs')} />
            </Field>
            <Field label="注册用户" required>
              <Input
                variant="bordered"
                value={String(draft.access_username ?? '')}
                onValueChange={(v) => set('access_username', v)}
              />
            </Field>
            <Field label="注册密码" required={!draft.has_access_password}>
              <Input
                type="password"
                variant="bordered"
                value={String(draft.access_password ?? '')}
                onValueChange={(v) => set('access_password', v)}
                placeholder={draft.has_access_password ? '用户名不变时留空表示不修改' : '请输入注册认证密码'}
              />
            </Field>
          </>
        )}
      </FormGrid>
      {showIp && (
        <IpRulesEditor rules={rules} onChange={setRules} />
      )}
    </div>
  );
}

function RegistrationTab({
  draft, set, registrations, endpoints, setEndpoints, onRefresh, refreshing,
}: {
  draft: Entity;
  set: (key: string, value: unknown) => void;
  registrations: Entity[];
  endpoints: EgressEndpoint[];
  setEndpoints: (endpoints: EgressEndpoint[]) => void;
  onRefresh?: () => void;
  refreshing?: boolean;
}) {
  const mode = String(draft.egress_connection_type ?? 'static_peer');
  const hasReg = registrations.length > 0;

  return (
    <div className="flex flex-col gap-5">
      <FormGrid>
        <Field label="连接方式" required>
          <Select
            variant="bordered"
            selectedKeys={[mode]}
            onChange={(e) => set('egress_connection_type', e.target.value)}
          >
            {connectionTypeOptions.map((opt) => (
              <SelectItem key={opt.value}>{opt.label}</SelectItem>
            ))}
          </Select>
        </Field>
        <Field label="传输协议" required>
          <Select variant="bordered" selectedKeys={['udp']} isDisabled>
            <SelectItem key="udp">UDP</SelectItem>
          </Select>
        </Field>
        {mode === 'client_register' && (
          <>
            <Field label="注册服务器" required>
              <Input
                variant="bordered"
                value={String(draft.register_server ?? '')}
                onValueChange={(v) => set('register_server', v)}
              />
            </Field>
            <Field label="注册用户" required>
              <Input
                variant="bordered"
                value={String(draft.register_username ?? '')}
                onValueChange={(v) => set('register_username', v)}
              />
            </Field>
            <Field label="注册密码" required={!draft.has_register_password}>
              <Input
                type="password"
                variant="bordered"
                value={String(draft.register_password ?? '')}
                onValueChange={(v) => set('register_password', v)}
                placeholder={draft.has_register_password ? '留空表示不修改' : ''}
              />
            </Field>
            <Field label="注册周期">
              <Input
                type="number"
                variant="bordered"
                value={String(draft.register_refresh_secs ?? 300)}
                onValueChange={(v) => set('register_refresh_secs', Number(v) || 300)}
                min={60}
              />
            </Field>
          </>
        )}
      </FormGrid>
      {mode === 'static_peer' && <EgressEndpointsEditor endpoints={endpoints} onChange={setEndpoints} />}
      <SectionBlock
        title="注册状态"
        actions={
          <div className="flex items-center gap-2">
            <Chip
              size="sm"
              variant="flat"
              color={hasReg ? 'success' : 'default'}
              startContent={<span className={`w-2 h-2 rounded-full ${hasReg ? 'bg-success animate-pulse' : 'bg-default-400'}`} />}
            >
              {hasReg ? '已有注册 (正常)' : '暂无注册'}
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
              <TableColumn key="node">来源 Socket / 节点</TableColumn>
              <TableColumn key="status">状态</TableColumn>
              <TableColumn key="expires_at">过期时间</TableColumn>
            </TableHeader>
            <TableBody items={registrations}>
              {(row) => (
                <TableRow key={String(row.contact_uri ?? row.contact ?? row.id ?? row.user_agent ?? '')}>
                  <TableCell className="font-mono text-tiny">{String(row.contact_uri ?? row.contact ?? '')}</TableCell>
                  <TableCell>{String(row.received_from ?? row.node ?? '')}</TableCell>
                  <TableCell>
                    <Chip size="sm" variant="dot" color="success">在线</Chip>
                  </TableCell>
                  <TableCell>{String(row.expires_at ?? '')}</TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        ) : (
          <p className="text-tiny text-default-400">暂无注册终端</p>
        )}
      </SectionBlock>
    </div>
  );
}

function AccessRegistrationStatus({
  registrations,
  onRefresh,
  refreshing,
}: {
  registrations: Entity[];
  onRefresh?: () => void;
  refreshing?: boolean;
}) {
  const hasReg = registrations.length > 0;
  return (
    <SectionBlock
      title="注册终端"
      description="仅注册认证或 IP 加认证模式会产生第三方注册状态。"
      actions={
        <div className="flex items-center gap-2">
          <Chip
            size="sm"
            variant="flat"
            color={hasReg ? 'success' : 'default'}
            startContent={<span className={`w-2 h-2 rounded-full ${hasReg ? 'bg-success animate-pulse' : 'bg-default-400'}`} />}
          >
            {hasReg ? `${registrations.length} 个在线注册` : '暂无注册'}
          </Chip>
          {onRefresh && (
            <Button
              size="sm"
              variant="flat"
              isLoading={refreshing}
              onPress={onRefresh}
              startContent={<RefreshCw className="w-3.5 h-3.5" />}
            >
              刷新状态
            </Button>
          )}
        </div>
      }
    >
      {registrations.length ? (
        <Table aria-label="注册终端列表">
          <TableHeader>
            <TableColumn key="contact">联系地址</TableColumn>
            <TableColumn key="node">来源 Socket / 节点</TableColumn>
            <TableColumn key="status">状态</TableColumn>
            <TableColumn key="expires_at">过期时间</TableColumn>
          </TableHeader>
          <TableBody items={registrations}>
            {(row) => (
              <TableRow key={String(row.contact_uri ?? row.contact ?? row.id ?? row.user_agent ?? '')}>
                <TableCell className="font-mono text-tiny">{String(row.contact_uri ?? row.contact ?? '')}</TableCell>
                <TableCell>{String(row.received_from ?? row.node ?? '')}</TableCell>
                <TableCell>
                  <Chip size="sm" variant="dot" color="success">在线</Chip>
                </TableCell>
                <TableCell>{String(row.expires_at ?? '')}</TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      ) : (
        <p className="text-tiny text-default-400">暂无第三方注册终端</p>
      )}
    </SectionBlock>
  );
}

export function CallerPolicyForm({ policy, set, pools, numbers }: { policy: OutboundPolicy; set: (key: keyof OutboundPolicy, value: unknown) => void; pools: Entity[]; numbers: Entity[] }) {
  const numberOptions = numbers.map((item) => ({ label: String(item.number), value: String(item.number) }));
  const poolOptions = pools.map((item) => ({ label: String(item.virtual_alias || item.id), value: String(item.id) }));
  return (
    <FormGrid>
      <Field label="主叫策略" required>
        <Select
          variant="bordered"
          selectedKeys={[policy.caller_mode]}
          onChange={(e) => set('caller_mode', e.target.value)}
        >
          {callerOptions.map((opt) => (
            <SelectItem key={opt.value}>{opt.label}</SelectItem>
          ))}
        </Select>
      </Field>
      <Field label="失败处理" required>
        <Select
          variant="bordered"
          selectedKeys={['reject']}
          isDisabled
          onChange={(e) => set('fallback_mode', e.target.value)}
        >
          <SelectItem key="reject">拒绝呼叫</SelectItem>
        </Select>
      </Field>
      {policy.caller_mode === 'fixed_number' && (
        <Field label="固定号码" required>
          <Select
            variant="bordered"
            selectedKeys={policy.fixed_number ? [String(policy.fixed_number)] : []}
            onChange={(e) => set('fixed_number', e.target.value)}
            placeholder="选择已授权真实号码"
          >
            {numberOptions.map((opt) => (
              <SelectItem key={opt.value}>{opt.label}</SelectItem>
            ))}
          </Select>
        </Field>
      )}
      {policy.caller_mode === 'virtual_pool' && (
        <Field label="主叫号码池" required>
          <Select
            variant="bordered"
            selectedKeys={policy.caller_pool_id ? [String(policy.caller_pool_id)] : []}
            onChange={(e) => set('caller_pool_id', e.target.value)}
            placeholder="选择当前来源的号码池"
          >
            {poolOptions.map((opt) => (
              <SelectItem key={opt.value}>{opt.label}</SelectItem>
            ))}
          </Select>
        </Field>
      )}
    </FormGrid>
  );
}

export function EgressBindingForm({ policy, set, groups, trunks }: { policy: OutboundPolicy; set: (key: keyof OutboundPolicy, value: unknown) => void; groups: Entity[]; trunks: Entity[] }) {
  const options = (items: Entity[]) => items.map((item) => ({ label: String(item.name ?? item.id), value: String(item.id) }));
  const egressTrunkOptions = options(trunks.filter((item) => trunkRole(item) === 'egress'));
  const groupOptions = options(groups);
  return (
    <FormGrid>
      <Field label="绑定方式" required>
        <Select
          variant="bordered"
          selectedKeys={[policy.egress_mode]}
          onChange={(e) => set('egress_mode', e.target.value)}
        >
          {egressModeOptions.map((opt) => (
            <SelectItem key={opt.value}>{opt.label}</SelectItem>
          ))}
        </Select>
      </Field>
      {policy.egress_mode === 'direct' ? (
        <Field label="落地中继" required>
          <Select
            variant="bordered"
            selectedKeys={policy.direct_egress_trunk_id ? [String(policy.direct_egress_trunk_id)] : []}
            onChange={(e) => set('direct_egress_trunk_id', e.target.value)}
            placeholder="选择唯一号码归属中继"
          >
            {egressTrunkOptions.map((opt) => (
              <SelectItem key={opt.value}>{opt.label}</SelectItem>
            ))}
          </Select>
        </Field>
      ) : (
        <Field label="落地分组" required>
          <Select
            variant="bordered"
            selectedKeys={policy.egress_group_id ? [String(policy.egress_group_id)] : []}
            onChange={(e) => set('egress_group_id', e.target.value)}
            placeholder="选择允许使用的落地范围"
          >
            {groupOptions.map((opt) => (
              <SelectItem key={opt.value}>{opt.label}</SelectItem>
            ))}
          </Select>
        </Field>
      )}
    </FormGrid>
  );
}

interface TabDef {
  key: string;
  title: string;
  content: ReactNode;
  hide?: boolean;
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
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const role = trunkRole(draft);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const workspace = await getTrunkWorkspace(id);
      const workspaceRole = trunkRole(workspace.trunk);
      setData(workspace);
      setDraft({
        ...workspace.trunk,
        role: workspaceRole,
        egress_connection_type: workspace.trunk.supports_registration ? 'client_register' : 'static_peer',
        register_server: workspace.trunk.host,
        register_username: workspace.trunk.reg_username,
      });
      setRules([]);
      setEndpoints([]);
      setPolicy(emptyPolicy);
      setGroups([]);
      setPools([]);
      setTrunks([]);
      if (workspaceRole === 'access') {
        const loadedRules = await getTrunkIpRules(id);
        setRules(loadedRules.map((rule) => ({ ...rule, _key: rule.id || genId() })));
        try {
          setPolicy({ ...emptyPolicy, ...await getOutboundPolicy(id) });
        } catch (reason) {
          if (!(reason instanceof Error && 'status' in reason && reason.status === 404)) throw reason;
        }
      } else {
        const loadedEndpoints = await getTrunkEgressEndpoints(id);
        setEndpoints(loadedEndpoints.map((ep) => ({ ...ep, _key: ep.id || genId() })));
      }
      const optional = await Promise.allSettled([
        listOptions('/egress-groups'),
        listOptions('/trunks'),
        listOptions('/caller-pools'),
      ]);
      if (optional[0].status === 'fulfilled') setGroups(optional[0].value);
      if (optional[1].status === 'fulfilled') setTrunks(optional[1].value);
      if (optional[2].status === 'fulfilled') setPools(optional[2].value.filter((pool) => pool.owner_source_type === 'trunk' && pool.owner_source_id === id));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '中继加载失败');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => { void load(); }, [load]);

  const set = (key: string, value: unknown) => setDraft((current) => ({ ...current, [key]: value }));
  const setPolicyField = (key: keyof OutboundPolicy, value: unknown) => setPolicy((current) => ({ ...current, [key]: value }));

  const save = async () => {
    const trunkError = trunkValidationError(draft, data?.trunk || {}, rules, endpoints);
    if (trunkError) { message.error(trunkError); return; }
    const policyError = role === 'access' ? policyValidationError(policy) : null;
    if (policyError) { message.error(policyError); return; }
    try {
      setSaving(true);
      const body: Entity = { ...draft };
      if (role === 'access') {
        body.supports_registration = ['digest_register', 'ip_and_digest'].includes(String(draft.access_auth_mode));
        body.reg_auth_type = String(draft.access_auth_mode).includes('digest') ? 'digest' : 'ip';
      } else if (role === 'egress') {
        const connectionType = draft.egress_connection_type ?? 'static_peer';
        if (connectionType === 'client_register') {
          body.supports_registration = true;
          body.reg_auth_type = 'digest';
          body.reg_username = draft.register_username;
          body.host = draft.register_server;
          if (typeof draft.register_password === 'string' && draft.register_password.trim()) {
            body.reg_password = draft.register_password;
          }
        } else {
          body.supports_registration = false;
          body.reg_auth_type = 'none';
          body.reg_password = '';
        }
      }
      if (role === 'access') {
        const needsIp = ['ip_allowlist', 'ip_and_digest'].includes(String(draft.access_auth_mode));
        const currentlyNeedsIp = ['ip_allowlist', 'ip_and_digest'].includes(String(data?.trunk.access_auth_mode));
        // Entering IP authentication stores rules first. Leaving IP-only
        // authentication changes the mode first so the API can safely remove
        // the final allowlist rule without creating an invalid intermediate state.
        if (needsIp || !currentlyNeedsIp || rules.some((rule) => rule.enabled)) {
          await saveTrunkIpRules(id, rules);
          await updateTrunk(id, body);
        } else {
          await updateTrunk(id, body);
          await saveTrunkIpRules(id, rules);
        }
        await saveOutboundPolicy(id, policy);
      } else if (role === 'egress') {
        await updateTrunk(id, body);
        await saveTrunkEgressEndpoints(id, endpoints);
      }
      message.success('中继配置已保存');
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '保存失败');
    } finally {
      setSaving(false);
    }
  };

  const tabs = useMemo<TabDef[]>(() => {
    const list: TabDef[] = [
      { key: 'basic', title: '基本配置', content: <BasicTab draft={draft} set={set} /> },
      {
        key: 'auth',
        title: '接入认证',
        content: <AccessAuthTab draft={draft} set={set} rules={rules} setRules={setRules} />,
        hide: role !== 'access',
      },
      {
        key: 'registration',
        title: '注册状态',
        content: role === 'egress'
          ? <RegistrationTab draft={draft} set={set} registrations={data?.registrations || []} endpoints={endpoints} setEndpoints={setEndpoints} onRefresh={load} refreshing={loading} />
          : <AccessRegistrationStatus registrations={data?.registrations || []} onRefresh={load} refreshing={loading} />,
      },
      {
        key: 'caller',
        title: '主叫策略',
        content: <CallerPolicyForm policy={policy} set={setPolicyField} pools={pools} numbers={data?.numbers || []} />,
        hide: role !== 'access',
      },
      {
        key: 'pool',
        title: role === 'access' ? '号码池组' : '归属号码',
        content: role === 'access'
          ? (
            <SectionBlock
              title="号码池组"
              actions={<Chip size="sm" variant="flat">{policy.caller_mode === 'virtual_pool' ? (policy.caller_pool_id || '尚未绑定') : '当前策略不使用号码池'}</Chip>}
            >
              <p className="text-tiny text-default-400">号码池成员是唯一归属于落地中继的真实号码。请在"号码池组"页面维护成员和选号算法。</p>
            </SectionBlock>
          )
          : (
            <SectionBlock title="归属号码">
              {data?.numbers?.length ? (
                <Table aria-label="归属号码列表">
                  <TableHeader>
                    <TableColumn key="number">真实号码</TableColumn>
                    <TableColumn key="can_present">可做主叫</TableColumn>
                    <TableColumn key="can_receive">可接呼入</TableColumn>
                  </TableHeader>
                  <TableBody items={data.numbers}>
                    {(row) => (
                      <TableRow key={String(row.number ?? row.id ?? row.owner_egress_trunk_id ?? '')}>
                        <TableCell>{String(row.number ?? '')}</TableCell>
                        <TableCell>{row.can_present ? '是' : '否'}</TableCell>
                        <TableCell>{row.can_receive ? '是' : '否'}</TableCell>
                      </TableRow>
                    )}
                  </TableBody>
                </Table>
              ) : (
                <p className="text-tiny text-default-400">该落地中继暂无归属号码</p>
              )}
            </SectionBlock>
          ),
      },
      {
        key: 'binding',
        title: '落地绑定',
        content: <EgressBindingForm policy={policy} set={setPolicyField} groups={groups} trunks={trunks} />,
        hide: role !== 'access',
      },
    ];
    return list.filter((tab) => !tab.hide);
  }, [data, draft, groups, policy, pools, role, rules, trunks, endpoints, load, loading]);

  if (loading) {
    return (
      <section>
        <DetailHeader loading={true} saving={saving} onRefresh={load} onSave={save} />
        <DetailLoading />
      </section>
    );
  }

  const isEnabled = draft.enabled !== false;
  const regCount = data?.registrations?.length ?? 0;
  const isOnline = regCount > 0;

  return (
    <section className="flex flex-col gap-4">
      {/* 顶集中继状态概览栏 */}
      <div className="flex flex-wrap items-center justify-between gap-4 p-4 bg-content1 rounded-xl border border-default-200">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center text-primary">
            <Server className="w-5 h-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-bold text-foreground">中继 {id}</h2>
              <Chip size="sm" variant="flat" color={role === 'access' ? 'primary' : 'default'}>
                {role === 'access' ? '接入中继' : '落地中继'}
              </Chip>
              <Chip size="sm" variant="flat" color={isEnabled ? 'success' : 'danger'}>
                {isEnabled ? '已启用' : '已禁用'}
              </Chip>
              <Chip
                size="sm"
                variant="flat"
                color={isOnline ? 'success' : 'default'}
                startContent={<span className={`w-2 h-2 rounded-full ${isOnline ? 'bg-success animate-pulse' : 'bg-default-400'}`} />}
              >
                {isOnline ? `在线 (${regCount} 个注册)` : '未注册/离线'}
              </Chip>
            </div>
            <p className="text-tiny text-default-400 mt-0.5">SIP 中继对接、IP/Digest 鉴权与网关链路属性配置</p>
          </div>
        </div>

        <DetailHeader loading={loading} saving={saving} onRefresh={load} onSave={save} />
      </div>

      {error ? (
        <DetailErrorState error={error} />
      ) : (
        <Card>
          <CardBody className="p-6">
            <Tabs aria-label="中继配置">
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
