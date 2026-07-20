import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Button, Card, CardBody, Input, Select, SelectItem, Switch,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Tabs, Tab,
} from '@heroui/react';
import { Plus, Trash2 } from 'lucide-react';
import { useParams } from 'react-router-dom';
import type { Entity } from '@/services/resources';
import { listOptions } from '@/services/trunks';
import { callerPoolValidationError, getCallerPool, getCallerPoolMembers, numberCanJoinCallerPool, saveCallerPoolMembers, updateCallerPool, type CallerPool, type CallerPoolMember } from '@/services/caller-pools';
import { trunkRole } from '@/services/trunks';
import { WorkspaceField } from '@/pages/trunks/trunk-detail';
import { DetailErrorState, DetailHeader, DetailLoading, FormGrid, SectionBlock } from '@/components/detail-shell';
import { message } from '@/utils/toast';

const strategyOptions = [
  { label: '均匀随机', value: 'random' },
  { label: '权重随机', value: 'weighted_random' },
  { label: '顺序轮询', value: 'round_robin' },
  { label: '稳定哈希', value: 'stable_hash' },
];

const sourceTypeOptions = [
  { label: '接入中继', value: 'trunk' },
  { label: '分机号码', value: 'extension' },
];

const genId = () => (crypto.randomUUID ? crypto.randomUUID() : Math.random().toString(36).substring(2));

interface TabDef {
  key: string;
  title: string;
  content: ReactNode;
}

export default function CallerPoolDetailPage() {
  const { id = '' } = useParams();
  const [pool, setPool] = useState<CallerPool | null>(null);
  const [members, setMembers] = useState<CallerPoolMember[]>([]);
  const [numbers, setNumbers] = useState<Entity[]>([]);
  const [accessTrunks, setAccessTrunks] = useState<Entity[]>([]);
  const [extensions, setExtensions] = useState<Entity[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [poolData, membersData, numbersData, trunksData, extensionsData] = await Promise.all([
        getCallerPool(id),
        getCallerPoolMembers(id),
        listOptions('/numbers'),
        listOptions('/trunks'),
        listOptions('/extensions'),
      ]);
      setPool(poolData);
      setMembers(membersData.map((m) => ({ ...m, _key: genId() })));
      setNumbers(numbersData);
      setAccessTrunks(trunksData.filter((trunk) => trunkRole(trunk) === 'access'));
      setExtensions(extensionsData);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '号码池加载失败');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => { void load(); }, [load]);

  const save = async () => {
    if (!pool) return;
    const validationError = callerPoolValidationError(pool, members);
    if (validationError) { message.error(validationError); return; }
    try {
      setSaving(true);
      await Promise.all([
        updateCallerPool(id, pool),
        saveCallerPoolMembers(id, members),
      ]);
      message.success('号码池配置已保存');
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '保存失败');
    } finally {
      setSaving(false);
    }
  };

  const patchMember = (index: number, values: Partial<CallerPoolMember>) => {
    setMembers((current) =>
      current.map((item, itemIndex) => (itemIndex === index ? { ...item, ...values } : item))
    );
  };

  const numberOptions = useMemo(() => {
    if (!pool) return [];
    const selected = new Set(members.map((member) => member.number));
    return numbers
      .filter((number) => numberCanJoinCallerPool(number, pool) || selected.has(String(number.number)))
      .map((number) => ({ label: String(number.number), value: String(number.number) }));
  }, [members, numbers, pool]);

  const sourceOptions = useMemo(() => {
    if (!pool) return [];
    if (pool.owner_source_type === 'trunk') return accessTrunks.map((trunk) => ({ label: String(trunk.name ?? trunk.id), value: String(trunk.id) }));
    if (pool.owner_source_type === 'extension') return extensions.map((extension) => ({ label: String(extension.display_name ?? extension.username), value: String(extension.username) }));
    return [];
  }, [accessTrunks, extensions, pool]);

  const tabs = useMemo<TabDef[]>(() => {
    if (!pool) return [];
    return [
      {
        key: 'basic',
        title: '基本配置',
        content: (
          <FormGrid>
            <WorkspaceField label="号码池 ID" required>
              <Input variant="bordered" value={pool.id} isDisabled />
            </WorkspaceField>
            <WorkspaceField label="虚拟主叫别名" required>
              <Input
                variant="bordered"
                value={pool.virtual_alias}
                onValueChange={(v) => setPool((curr) => curr ? { ...curr, virtual_alias: v } : null)}
              />
            </WorkspaceField>
            <WorkspaceField label="来源类型" required>
              <Select
                variant="bordered"
                selectedKeys={[pool.owner_source_type]}
                onChange={(e) => setPool((curr) => curr ? { ...curr, owner_source_type: e.target.value as CallerPool['owner_source_type'], owner_source_id: '' } : null)}
              >
                {sourceTypeOptions.map((opt) => (
                  <SelectItem key={opt.value}>{opt.label}</SelectItem>
                ))}
              </Select>
            </WorkspaceField>
            <WorkspaceField label="来源标识" required>
              <Select
                variant="bordered"
                selectedKeys={pool.owner_source_id ? [String(pool.owner_source_id)] : []}
                onChange={(e) => setPool((curr) => curr ? { ...curr, owner_source_id: e.target.value } : null)}
                placeholder="请选择已存在的来源"
              >
                {sourceOptions.map((opt) => (
                  <SelectItem key={opt.value}>{opt.label}</SelectItem>
                ))}
              </Select>
            </WorkspaceField>
            <WorkspaceField label="选号算法" required>
              <Select
                variant="bordered"
                selectedKeys={[pool.strategy]}
                onChange={(e) => setPool((curr) => curr ? { ...curr, strategy: e.target.value as CallerPool['strategy'] } : null)}
              >
                {strategyOptions.map((opt) => (
                  <SelectItem key={opt.value}>{opt.label}</SelectItem>
                ))}
              </Select>
            </WorkspaceField>
            <WorkspaceField label="失败处理" required>
              <Select variant="bordered" selectedKeys={['reject']} isDisabled>
                <SelectItem key="reject">拒绝呼叫</SelectItem>
              </Select>
            </WorkspaceField>
            <WorkspaceField label="启用状态">
              <Switch
                isSelected={pool.enabled !== false}
                onChange={(e) => setPool((curr) => curr ? { ...curr, enabled: e.target.checked } : null)}
              />
            </WorkspaceField>
          </FormGrid>
        ),
      },
      {
        key: 'members',
        title: '号码池成员',
        content: (
          <SectionBlock
            title="真实号码成员"
            description="仅展示已授权给当前来源且允许显号的真实号码；历史成员仍会保留回显以便修正。"
            actions={
              <Button
                size="sm"
                variant="flat"
                onPress={() =>
                  setMembers((current) => [
                    ...current,
                    {
                      _key: genId(),
                      pool_id: id,
                      number: '',
                      priority: 100,
                      weight: 100,
                      max_concurrent: 0,
                      enabled: true,
                    },
                  ])
                }
                startContent={<Plus className="w-4 h-4" />}
              >
                添加号码
              </Button>
            }
          >
            <Table aria-label="号码池成员列表" isStriped>
              <TableHeader>
                <TableColumn key="number">真实号码</TableColumn>
                <TableColumn key="priority">优先级</TableColumn>
                <TableColumn key="weight">权重</TableColumn>
                <TableColumn key="max_concurrent">最大并发</TableColumn>
                <TableColumn key="enabled">启用</TableColumn>
                <TableColumn key="actions" align="end">操作</TableColumn>
              </TableHeader>
              <TableBody items={members} emptyContent="尚未配置号码池成员">
                {(row) => {
                  const idx = members.findIndex((m) => (m._key || m.id) === (row._key || row.id));
                  const rowKey = row._key || row.id || idx;
                  return (
                    <TableRow key={rowKey}>
                      <TableCell>
                        <Select
                          variant="underlined"
                          selectedKeys={row.number ? [String(row.number)] : []}
                          onChange={(e) => patchMember(idx, { number: e.target.value })}
                          placeholder="搜索或选择号码"
                        >
                          {numberOptions.map((opt) => (
                            <SelectItem key={opt.value}>{opt.label}</SelectItem>
                          ))}
                        </Select>
                      </TableCell>
                      <TableCell>
                        <Input
                          variant="underlined"
                          type="number"
                          value={String(row.priority ?? 100)}
                          onValueChange={(v) => patchMember(idx, { priority: Number(v) || 100 })}
                          min={0}
                          max={65535}
                        />
                      </TableCell>
                      <TableCell>
                        <Input
                          variant="underlined"
                          type="number"
                          value={String(row.weight ?? 100)}
                          onValueChange={(v) => patchMember(idx, { weight: Number(v) || 100 })}
                          min={1}
                          max={10000}
                        />
                      </TableCell>
                      <TableCell>
                        <Input
                          variant="underlined"
                          type="number"
                          value={String(row.max_concurrent ?? 0)}
                          onValueChange={(v) => patchMember(idx, { max_concurrent: Number(v) || 0 })}
                          placeholder="0 表示不限制"
                          min={0}
                        />
                      </TableCell>
                      <TableCell>
                        <Switch
                          size="sm"
                          isSelected={row.enabled !== false}
                          onChange={(e) => patchMember(idx, { enabled: e.target.checked })}
                        />
                      </TableCell>
                      <TableCell>
                        <Button
                          isIconOnly
                          size="sm"
                          color="danger"
                          variant="light"
                          onPress={() => setMembers((current) => current.filter((_, itemIndex) => itemIndex !== idx))}
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
        ),
      },
    ];
  }, [members, numberOptions, pool, sourceOptions, id]);

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
      ) : pool ? (
        <Card>
          <CardBody className="p-6">
            <Tabs aria-label="号码池配置">
              {tabs.map((tab) => (
                <Tab key={tab.key} title={tab.title}>
                  {tab.content}
                </Tab>
              ))}
            </Tabs>
          </CardBody>
        </Card>
      ) : null}
    </section>
  );
}
