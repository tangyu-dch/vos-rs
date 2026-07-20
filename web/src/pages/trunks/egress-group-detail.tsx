import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Button, Card, CardBody, Input, Select, SelectItem, Switch,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Tabs, Tab,
} from '@heroui/react';
import { Plus, Trash2 } from 'lucide-react';
import { useParams } from 'react-router-dom';
import type { Entity } from '@/services/resources';
import { listOptions } from '@/services/trunks';
import { egressGroupValidationError, getEgressGroup, getEgressGroupMembers, saveEgressGroupMembers, updateEgressGroup, type EgressGroup, type EgressGroupMember } from '@/services/egress-groups';
import { WorkspaceField } from '@/pages/trunks/trunk-detail';
import { DetailErrorState, DetailHeader, DetailLoading, FormGrid, SectionBlock } from '@/components/detail-shell';
import { message } from '@/utils/toast';

const genId = () => (crypto.randomUUID ? crypto.randomUUID() : Math.random().toString(36).substring(2));

interface TabDef {
  key: string;
  title: string;
  content: ReactNode;
}

export default function EgressGroupDetailPage() {
  const { id = '' } = useParams();
  const [group, setGroup] = useState<EgressGroup | null>(null);
  const [members, setMembers] = useState<EgressGroupMember[]>([]);
  const [trunks, setTrunks] = useState<Entity[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [groupData, membersData, trunksData] = await Promise.all([
        getEgressGroup(id),
        getEgressGroupMembers(id),
        listOptions('/trunks'),
      ]);
      setGroup(groupData);
      setMembers(membersData.map((m) => ({ ...m, _key: genId() })));
      setTrunks(trunksData.filter((t) => t.role === 'egress' || t.gateway_type === 'gateway'));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '落地分组加载失败');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => { void load(); }, [load]);

  const save = async () => {
    if (!group) return;
    const validationError = egressGroupValidationError(group, members);
    if (validationError) { message.error(validationError); return; }
    try {
      setSaving(true);
      await Promise.all([
        updateEgressGroup(id, group),
        saveEgressGroupMembers(id, members),
      ]);
      message.success('落地分组配置已保存');
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '保存失败');
    } finally {
      setSaving(false);
    }
  };

  const patchMember = (index: number, values: Partial<EgressGroupMember>) => {
    setMembers((current) =>
      current.map((item, itemIndex) => (itemIndex === index ? { ...item, ...values } : item))
    );
  };

  const trunkOptions = useMemo(() => trunks.map((t) => ({ label: String(t.id), value: String(t.id) })), [trunks]);

  const tabs = useMemo<TabDef[]>(() => {
    if (!group) return [];
    return [
      {
        key: 'basic',
        title: '基本配置',
        content: (
          <FormGrid>
            <WorkspaceField label="路由组 ID" required>
              <Input variant="bordered" value={group.id} isDisabled />
            </WorkspaceField>
            <WorkspaceField label="路由组名称" required>
              <Input
                variant="bordered"
                value={group.name}
                onValueChange={(v) => setGroup((curr) => curr ? { ...curr, name: v } : null)}
              />
            </WorkspaceField>
            <WorkspaceField label="分组说明" fullWidth>
              <Input
                variant="bordered"
                value={String(group.description ?? '')}
                onValueChange={(v) => setGroup((curr) => curr ? { ...curr, description: v } : null)}
              />
            </WorkspaceField>
            <WorkspaceField label="启用状态">
              <Switch
                isSelected={group.enabled !== false}
                onChange={(e) => setGroup((curr) => curr ? { ...curr, enabled: e.target.checked } : null)}
              />
            </WorkspaceField>
          </FormGrid>
        ),
      },
      {
        key: 'members',
        title: '落地中继成员',
        content: (
          <SectionBlock
            title="落地中继列表"
            description="按优先级与权重分配呼叫流量；当高优先级中继无可用并发或故障时，自动故障转移。"
            actions={
              <Button
                size="sm"
                variant="flat"
                onPress={() =>
                  setMembers((current) => [
                    ...current,
                    {
                      _key: genId(),
                      group_id: id,
                      egress_trunk_id: '',
                      destination_prefix: '',
                      priority: 100,
                      weight: 100,
                      time_start: null,
                      time_end: null,
                      enabled: true,
                    },
                  ])
                }
                startContent={<Plus className="w-4 h-4" />}
              >
                添加中继
              </Button>
            }
          >
            <Table aria-label="落地中继成员列表" isStriped>
              <TableHeader>
                <TableColumn key="egress_trunk_id">落地中继</TableColumn>
                <TableColumn key="destination_prefix">被叫前缀匹配</TableColumn>
                <TableColumn key="priority">优先级</TableColumn>
                <TableColumn key="weight">权重</TableColumn>
                <TableColumn key="time_start">开始时间</TableColumn>
                <TableColumn key="time_end">结束时间</TableColumn>
                <TableColumn key="enabled">启用</TableColumn>
                <TableColumn key="actions" align="end">操作</TableColumn>
              </TableHeader>
              <TableBody items={members} emptyContent="尚未配置落地中继成员">
                {(row) => {
                  const idx = members.findIndex((m) => (m._key || m.id) === (row._key || row.id));
                  const rowKey = row._key || row.id || idx;
                  return (
                    <TableRow key={rowKey}>
                      <TableCell>
                        <Select
                          variant="underlined"
                          selectedKeys={row.egress_trunk_id ? [String(row.egress_trunk_id)] : []}
                          onChange={(e) => patchMember(idx, { egress_trunk_id: e.target.value })}
                          placeholder="请选择"
                        >
                          {trunkOptions.map((opt) => (
                            <SelectItem key={opt.value}>{opt.label}</SelectItem>
                          ))}
                        </Select>
                      </TableCell>
                      <TableCell>
                        <Input
                          variant="underlined"
                          value={row.destination_prefix}
                          onValueChange={(v) => patchMember(idx, { destination_prefix: v })}
                          placeholder="可选，例如 86"
                        />
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
                          value={row.time_start || ''}
                          onValueChange={(v) => patchMember(idx, { time_start: v || null })}
                          placeholder="HH:MM"
                          maxLength={5}
                        />
                      </TableCell>
                      <TableCell>
                        <Input
                          variant="underlined"
                          value={row.time_end || ''}
                          onValueChange={(v) => patchMember(idx, { time_end: v || null })}
                          placeholder="HH:MM"
                          maxLength={5}
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
  }, [group, members, trunkOptions, id]);

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
      ) : group ? (
        <Card>
          <CardBody className="p-6">
            <Tabs aria-label="落地分组配置">
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
