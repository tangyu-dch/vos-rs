import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Alert, Button, Empty, Form, Grid, Input, InputNumber, Message, Select, Space, Spin,
  Switch, Table, Tabs
} from '@arco-design/web-react';
import { IconDelete, IconPlus, IconRefresh, IconSave } from '@arco-design/web-react/icon';
import { useParams } from 'react-router-dom';
import type { Entity } from '../services/resources';
import { listOptions } from '../services/trunks';
import { egressGroupValidationError, getEgressGroup, getEgressGroupMembers, saveEgressGroupMembers, updateEgressGroup, type EgressGroup, type EgressGroupMember } from '../services/egress-groups';
import { WorkspaceField } from './trunk-detail';

const genId = () => crypto.randomUUID ? crypto.randomUUID() : Math.random().toString(36).substring(2);

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
      // Filter only egress trunks/gateways
      setTrunks(trunksData.filter((t) => t.role === 'egress' || t.gateway_type === 'gateway'));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '落地分组加载失败');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    if (!group) return;
    const validationError = egressGroupValidationError(group, members);
    if (validationError) { Message.error(validationError); return; }

    try {
      setSaving(true);
      await Promise.all([
        updateEgressGroup(id, group),
        saveEgressGroupMembers(id, members),
      ]);
      Message.success('落地分组配置已保存');
      await load();
    } catch (reason) {
      Message.error(reason instanceof Error ? reason.message : '保存失败');
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

  const memberColumns = [
    {
      title: '落地中继',
      render: (_: unknown, row: EgressGroupMember, index: number) => (
        <Select
          value={row.egress_trunk_id}
          options={trunkOptions}
          onChange={(value) => patchMember(index, { egress_trunk_id: value })}
          placeholder="请选择"
        />
      ),
    },
    {
      title: '被叫前缀匹配',
      render: (_: unknown, row: EgressGroupMember, index: number) => (
        <Input
          value={row.destination_prefix}
          onChange={(value) => patchMember(index, { destination_prefix: value })}
          placeholder="可选，例如 86"
        />
      ),
    },
    {
      title: '优先级',
      width: 120,
      render: (_: unknown, row: EgressGroupMember, index: number) => (
        <InputNumber
          min={0}
          max={65535}
          precision={0}
          value={row.priority ?? 100}
          onChange={(value) => patchMember(index, { priority: Number(value ?? 100) })}
          style={{ width: '100%' }}
        />
      ),
    },
    {
      title: '权重',
      width: 120,
      render: (_: unknown, row: EgressGroupMember, index: number) => (
        <InputNumber
          min={1}
          max={10000}
          precision={0}
          value={row.weight ?? 100}
          onChange={(value) => patchMember(index, { weight: Number(value ?? 100) })}
          style={{ width: '100%' }}
        />
      ),
    },
    {
      title: '开始时间',
      width: 120,
      render: (_: unknown, row: EgressGroupMember, index: number) => (
        <Input
          value={row.time_start || ''}
          onChange={(value) => patchMember(index, { time_start: value || null })}
          placeholder="HH:MM"
          maxLength={5}
        />
      ),
    },
    {
      title: '结束时间',
      width: 120,
      render: (_: unknown, row: EgressGroupMember, index: number) => (
        <Input
          value={row.time_end || ''}
          onChange={(value) => patchMember(index, { time_end: value || null })}
          placeholder="HH:MM"
          maxLength={5}
        />
      ),
    },
    {
      title: '启用',
      width: 74,
      render: (_: unknown, row: EgressGroupMember, index: number) => (
        <Switch
          checked={row.enabled !== false}
          onChange={(value) => patchMember(index, { enabled: value })}
        />
      ),
    },
    {
      title: '',
      width: 58,
      render: (_: unknown, __: EgressGroupMember, index: number) => (
        <Button
          type="text"
          status="danger"
          icon={<IconDelete />}
          aria-label="删除成员"
          onClick={() => setMembers((current) => current.filter((_, itemIndex) => itemIndex !== index))}
        />
      ),
    },
  ];

  const tabs = useMemo(() => {
    if (!group) return [];
    return [
      {
        key: 'basic',
        title: '基本配置',
        content: (
          <Form layout="vertical">
            <Grid.Row className="form-grid" gutter={[18, 0]}>
              <WorkspaceField label="分组 ID" required>
                <Input value={group.id} disabled />
              </WorkspaceField>
              <WorkspaceField label="分组名称" required>
                <Input
                  value={group.name}
                  onChange={(value) => setGroup((curr) => curr ? { ...curr, name: value } : null)}
                />
              </WorkspaceField>
              <WorkspaceField label="启用状态">
                <Switch
                  checked={group.enabled !== false}
                  onChange={(value) => setGroup((curr) => curr ? { ...curr, enabled: value } : null)}
                  checkedText="启用"
                  uncheckedText="停用"
                />
              </WorkspaceField>
            </Grid.Row>
          </Form>
        ),
      },
      {
        key: 'members',
        title: '落地中继成员',
        content: (
          <div className="repeat-editor">
            <div className="section-title">
              <div>
                <h2>落地中继选择</h2>
                <p className="muted-copy">按被叫前缀、业务时段、优先级和权重选择落地中继；相同规则不能重复。</p>
              </div>
              <Button
                icon={<IconPlus />}
                onClick={() =>
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
              >
                添加成员
              </Button>
            </div>
            <Table
              rowKey={(record) => record._key || String(record.id || '')}
              pagination={false}
              data={members}
              columns={memberColumns}
              scroll={{ x: 1000 }}
              noDataElement={<Empty description="尚未配置落地中继成员" />}
            />
          </div>
        ),
      },
    ];
  }, [group, members, trunkOptions]);

  return (
    <section className="workspace">
      <header className="page-header">
        <div>
          <h1>{group?.name || '落地分组详情'}</h1>
          <p>管理落地组网关优先级、权重及业务时段。</p>
        </div>
        <Space>
          <Button icon={<IconRefresh />} loading={loading} onClick={load}>
            刷新
          </Button>
          <Button type="primary" icon={<IconSave />} loading={saving} onClick={save}>
            保存配置
          </Button>
        </Space>
      </header>
      {error ? (
        <Alert type="error" title="数据加载失败" content={error} />
      ) : (
        <Spin loading={loading} block>
          {group && (
            <div className="trunk-workspace">
              <Tabs defaultActiveTab="basic">
                {tabs.map((tab) => (
                  <Tabs.TabPane key={tab.key} title={tab.title}>
                    {tab.content}
                  </Tabs.TabPane>
                ))}
              </Tabs>
            </div>
          )}
        </Spin>
      )}
    </section>
  );
}
