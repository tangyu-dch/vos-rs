import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Alert, Button, Empty, Form, Grid, Input, Message, Select, Space, Spin,
  Switch, Table, Tabs
} from '@arco-design/web-react';
import { IconDelete, IconPlus, IconRefresh, IconSave } from '@arco-design/web-react/icon';
import { useParams } from 'react-router-dom';
import type { Entity } from '../services/resources';
import { listOptions } from '../services/trunks';
import { getCallerPool, getCallerPoolMembers, saveCallerPoolMembers, updateCallerPool, type CallerPool, type CallerPoolMember } from '../services/caller-pools';
import { WorkspaceField } from './trunk-detail';

const strategyOptions = [
  { label: '均匀随机', value: 'random' },
  { label: '权重随机', value: 'weighted_random' },
  { label: '顺序轮询', value: 'round_robin' },
  { label: '稳定哈希', value: 'stable_hash' },
];

const fallbackOptions = [
  { label: '拒绝呼叫', value: 'reject' },
  { label: '固定替换', value: 'fixed' },
  { label: '号码池替换', value: 'pool' },
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
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [poolData, membersData, numbersData] = await Promise.all([
        getCallerPool(id),
        getCallerPoolMembers(id),
        listOptions('/numbers'),
      ]);
      setPool(poolData);
      setMembers(membersData.map((m) => ({ ...m, _key: genId() })));
      setNumbers(numbersData);
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
    if (!pool.virtual_alias?.trim() || !pool.owner_source_id?.trim()) {
      Message.error('虚拟主叫与来源标识不能为空');
      return;
    }
    // Validation for members
    for (const member of members) {
      if (!member.number?.trim()) {
        Message.error('号码不能为空');
        return;
      }
    }

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

  const numberOptions = useMemo(() => numbers.map((n) => ({ label: String(n.number), value: String(n.number) })), [numbers]);

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
        <Input
          type="number"
          value={row.priority !== undefined ? String(row.priority) : ''}
          onChange={(value) => patchMember(index, { priority: value ? Number(value) : 100 })}
        />
      ),
    },
    {
      title: '权重',
      width: 120,
      render: (_: unknown, row: CallerPoolMember, index: number) => (
        <Input
          type="number"
          value={row.weight !== undefined ? String(row.weight) : ''}
          onChange={(value) => patchMember(index, { weight: value ? Number(value) : 100 })}
        />
      ),
    },
    {
      title: '最大并发',
      width: 140,
      render: (_: unknown, row: CallerPoolMember, index: number) => (
        <Input
          type="number"
          value={row.max_concurrent !== undefined ? String(row.max_concurrent) : ''}
          onChange={(value) => patchMember(index, { max_concurrent: value ? Number(value) : 0 })}
          placeholder="0 表示不限制"
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
                  onChange={(value) => setPool((curr) => curr ? { ...curr, owner_source_type: value } : null)}
                />
              </WorkspaceField>
              <WorkspaceField label="来源标识" required>
                <Input
                  value={pool.owner_source_id}
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
                <h2>号码池分机绑定</h2>
                <p className="muted-copy">在此加入用于呼出轮询、显号的真实物理号码及每个号码的路由权重。</p>
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
  }, [pool, members, numberOptions]);

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
