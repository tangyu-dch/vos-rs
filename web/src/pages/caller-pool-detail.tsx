import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Alert, Button, Empty, Form, Grid, Input, InputNumber, Message, Select, Space, Spin,
  Switch, Table, Tabs
} from '@arco-design/web-react';
import { IconDelete, IconPlus, IconRefresh, IconSave } from '@arco-design/web-react/icon';
import { useParams } from 'react-router-dom';
import type { Entity } from '../services/resources';
import { listOptions } from '../services/trunks';
import { callerPoolValidationError, getCallerPool, getCallerPoolMembers, numberCanJoinCallerPool, saveCallerPoolMembers, updateCallerPool, type CallerPool, type CallerPoolMember } from '../services/caller-pools';
import { trunkRole } from '../services/trunks';
import { WorkspaceField } from './trunk-detail';

const strategyOptions = [
  { label: '均匀随机', value: 'random' },
  { label: '权重随机', value: 'weighted_random' },
  { label: '顺序轮询', value: 'round_robin' },
  { label: '稳定哈希', value: 'stable_hash' },
];

const fallbackOptions = [
  { label: '拒绝呼叫', value: 'reject' },
];

const sourceTypeOptions = [
  { label: '接入中继', value: 'trunk' },
  { label: '分机号码', value: 'extension' },
  { label: '分机群组', value: 'extension_group' },
];

const genId = () => crypto.randomUUID ? crypto.randomUUID() : Math.random().toString(36).substring(2);

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

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    if (!pool) return;
    const validationError = callerPoolValidationError(pool, members);
    if (validationError) { Message.error(validationError); return; }

    try {
      setSaving(true);
      await Promise.all([
        updateCallerPool(id, pool),
        saveCallerPoolMembers(id, members),
      ]);
      Message.success('号码池配置已保存');
      await load();
    } catch (reason) {
      Message.error(reason instanceof Error ? reason.message : '保存失败');
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
    return pool.owner_source_id ? [{ label: pool.owner_source_id, value: pool.owner_source_id }] : [];
  }, [accessTrunks, extensions, pool]);

  const memberColumns = [
    {
      title: '真实号码',
      render: (_: unknown, row: CallerPoolMember, index: number) => (
        <Select
          showSearch
          value={row.number}
          options={numberOptions}
          onChange={(value) => patchMember(index, { number: value })}
          placeholder="搜索或选择号码"
        />
      ),
    },
    {
      title: '优先级',
      width: 120,
      render: (_: unknown, row: CallerPoolMember, index: number) => (
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
      render: (_: unknown, row: CallerPoolMember, index: number) => (
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
      title: '最大并发',
      width: 140,
      render: (_: unknown, row: CallerPoolMember, index: number) => (
        <InputNumber
          min={0}
          precision={0}
          value={row.max_concurrent ?? 0}
          onChange={(value) => patchMember(index, { max_concurrent: Number(value ?? 0) })}
          placeholder="0 表示不限制"
          style={{ width: '100%' }}
        />
      ),
    },
    {
      title: '启用',
      width: 74,
      render: (_: unknown, row: CallerPoolMember, index: number) => (
        <Switch
          checked={row.enabled !== false}
          onChange={(value) => patchMember(index, { enabled: value })}
        />
      ),
    },
    {
      title: '',
      width: 58,
      render: (_: unknown, __: CallerPoolMember, index: number) => (
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
    if (!pool) return [];
    return [
      {
        key: 'basic',
        title: '基本配置',
        content: (
          <Form layout="vertical">
            <Grid.Row className="form-grid" gutter={[18, 0]}>
              <WorkspaceField label="号码池 ID" required>
                <Input value={pool.id} disabled />
              </WorkspaceField>
              <WorkspaceField label="虚拟主叫别名" required>
                <Input
                  value={pool.virtual_alias}
                  onChange={(value) => setPool((curr) => curr ? { ...curr, virtual_alias: value } : null)}
                />
              </WorkspaceField>
              <WorkspaceField label="来源类型" required>
                <Select
                  value={pool.owner_source_type}
                  options={sourceTypeOptions}
                  onChange={(value) => setPool((curr) => curr ? { ...curr, owner_source_type: value, owner_source_id: '' } : null)}
                />
              </WorkspaceField>
              <WorkspaceField label="来源标识" required>
                <Select
                  showSearch
                  allowCreate={pool.owner_source_type === 'extension_group'}
                  value={pool.owner_source_id}
                  options={sourceOptions}
                  placeholder={pool.owner_source_type === 'extension_group' ? '选择或录入分机群组' : '请选择已存在的来源'}
                  onChange={(value) => setPool((curr) => curr ? { ...curr, owner_source_id: value } : null)}
                />
              </WorkspaceField>
              <WorkspaceField label="选号算法" required>
                <Select
                  value={pool.strategy}
                  options={strategyOptions}
                  onChange={(value) => setPool((curr) => curr ? { ...curr, strategy: value } : null)}
                />
              </WorkspaceField>
              <WorkspaceField label="失败处理" required>
                <Select
                  value={pool.fallback_mode}
                  options={fallbackOptions}
                  onChange={(value) => setPool((curr) => curr ? { ...curr, fallback_mode: value } : null)}
                />
              </WorkspaceField>
              <WorkspaceField label="启用状态">
                <Switch
                  checked={pool.enabled !== false}
                  onChange={(value) => setPool((curr) => curr ? { ...curr, enabled: value } : null)}
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
        title: '号码池成员',
        content: (
          <div className="repeat-editor">
            <div className="section-title">
              <div>
                <h2>真实号码成员</h2>
                <p className="muted-copy">仅展示已授权给当前来源且允许显号的真实号码；历史成员仍会保留回显以便修正。</p>
              </div>
              <Button
                icon={<IconPlus />}
                onClick={() =>
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
              >
                添加号码
              </Button>
            </div>
            <Table
              rowKey={(record) => record._key || String(record.id || '')}
              pagination={false}
              data={members}
              columns={memberColumns}
              scroll={{ x: 800 }}
              noDataElement={<Empty description="尚未配置号码池成员" />}
            />
          </div>
        ),
      },
    ];
  }, [members, numberOptions, pool, sourceOptions]);

  return (
    <section className="workspace">
      <header className="page-header">
        <div>
          <h1>{pool?.virtual_alias || '号码池详情'}</h1>
          <p>管理外呼号码组的主叫隐藏、并发控制及轮询分配策略。</p>
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
          {pool && (
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
