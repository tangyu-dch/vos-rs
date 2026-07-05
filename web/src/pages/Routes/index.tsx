import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Button,
  Space,
  Modal,
  Form,
  Input,
  InputNumber,
  Select,
  Message,
  Popconfirm,
  Alert,
  Empty,
} from '@arco-design/web-react';
import { IconPlus, IconEdit, IconDelete, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { SipRoute, SipGateway } from '@/types';

const FormItem = Form.Item;

export default function RoutesPage() {
  const [routes, setRoutes] = useState<SipRoute[]>([]);
  const [gateways, setGateways] = useState<SipGateway[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modalVisible, setModalVisible] = useState(false);
  const [editingRoute, setEditingRoute] = useState<SipRoute | null>(null);
  const [form] = Form.useForm();
  const [previewDest, setPreviewDest] = useState('');
  const [previewResult, setPreviewResult] = useState<{
    candidates: { route_id: string; gateway_id: string; host: string; port: number | null }[];
    error?: string;
  } | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);

  const handlePreview = async () => {
    if (!previewDest.trim()) return;
    setPreviewLoading(true);
    try {
      setPreviewResult(await apiService.routePreview(previewDest.trim()));
    } catch {
      setPreviewResult({ candidates: [], error: '查询失败（sip-edge 管理 API 未启用？）' });
    } finally {
      setPreviewLoading(false);
    }
  };

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [r, g] = await Promise.all([apiService.getRoutes(), apiService.getGateways()]);
      setRoutes(r);
      setGateways(g);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取数据失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleAdd = () => {
    setEditingRoute(null);
    form.resetFields();
    setModalVisible(true);
  };

  const handleEdit = (route: SipRoute) => {
    setEditingRoute(route);
    form.setFieldsValue(route);
    setModalVisible(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await apiService.deleteRoute(id);
      Message.success('删除成功');
      loadData();
    } catch {
      Message.error('删除失败');
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      if (editingRoute) {
        await apiService.updateRoute(editingRoute.id, values);
        Message.success('更新成功');
      } else {
        await apiService.createRoute(values);
        Message.success('创建成功');
      }
      setModalVisible(false);
      loadData();
    } catch {
      /* 校验失败 */
    }
  };

  const getGateway = (id: string) => {
    const g = gateways.find((x) => x.id === id);
    return g ? `${g.id} · ${g.host}` : id;
  };

  const columns = [
    {
      title: '路由 ID',
      dataIndex: 'id',
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
    },
    {
      title: '前缀',
      dataIndex: 'prefix',
      width: 120,
      render: (v: string) => <span className="prefix-tag">{v}</span>,
    },
    {
      title: '优先级',
      dataIndex: 'priority',
      width: 90,
      render: (v: number) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '目标网关',
      dataIndex: 'gateway_id',
      render: (v: string) => <span className="cell-mono">{getGateway(v)}</span>,
    },
    {
      title: '成本',
      dataIndex: 'cost',
      width: 110,
      render: (v: number) => <span className="cell-mono">¥{v.toFixed(4)}</span>,
    },
    {
      title: '时间窗口',
      dataIndex: 'time_start',
      width: 150,
      render: (_: any, r: SipRoute) =>
        r.time_start && r.time_end ? (
          <span className="cell-mono">
            {r.time_start}~{r.time_end}
          </span>
        ) : (
          <span className="cell-dash">不限</span>
        ),
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      render: (d: string) => (d ? new Date(d).toLocaleString('zh-CN') : '—'),
    },
    {
      title: '操作',
      dataIndex: 'actions',
      width: 180,
      fixed: 'right' as const,
      render: (_: any, record: SipRoute) => (
        <Space size={4}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm title="确认删除该路由？" icon={null} onOk={() => handleDelete(record.id)}>
            <Button type="text" size="small" status="danger" icon={<IconDelete />}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>路由管理</h1>
          <span className="sub">按号前缀与优先级配置出局选路规则（LCR）</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={loadData}>
            刷新
          </Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            新建路由
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false} style={{ marginBottom: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
          <span style={{ fontWeight: 600, fontSize: 14 }}>选路试算</span>
          <Input
            placeholder="输入被叫号码，查看候选路由（failover 顺序）"
            value={previewDest}
            onChange={setPreviewDest}
            style={{ width: 320 }}
            allowClear
          />
          <Button type="primary" loading={previewLoading} onClick={handlePreview}>
            试算
          </Button>
        </div>
        {previewResult && (
          <div>
            {previewResult.error ? (
              <span className="cell-dash">{previewResult.error}</span>
            ) : previewResult.candidates.length === 0 ? (
              <span className="cell-dash">无匹配路由</span>
            ) : (
              previewResult.candidates.map((c, i) => (
                <div key={c.route_id} style={{ marginBottom: 6 }}>
                  <span className="prefix-tag">{i + 1}</span>{' '}
                  <span className="cell-mono cell-strong">{c.route_id}</span>
                  {' → '}
                  <span className="cell-mono">
                    {c.gateway_id} ({c.host}
                    {c.port ? `:${c.port}` : ''})
                  </span>
                </div>
              ))
            )}
          </div>
        )}
      </Card>

      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={routes}
          rowKey="id"
          loading={loading}
          pagination={{ pageSize: 20, sizeCanChange: true }}
          scroll={{ x: 980 }}
          noDataElement={<Empty description="暂无路由规则" />}
        />
      </Card>

      <Modal
        title={editingRoute ? '编辑路由' : '新建路由'}
        visible={modalVisible}
        onOk={handleSubmit}
        onCancel={() => setModalVisible(false)}
        okText="保存"
        cancelText="取消"
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem label="路由 ID" field="id" required rules={[{ required: true, message: '请输入路由 ID' }]}>
            <Input placeholder="如 route1" disabled={!!editingRoute} />
          </FormItem>
          <FormItem label="前缀" field="prefix" required rules={[{ required: true, message: '请输入前缀' }]}>
            <Input placeholder="如 138" />
          </FormItem>
          <FormItem label="优先级" field="priority" required rules={[{ required: true, message: '请输入优先级' }]}>
            <InputNumber placeholder="10" min={0} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="目标网关" field="gateway_id" required rules={[{ required: true, message: '请选择网关' }]}>
            <Select placeholder="请选择">
              {gateways.map((g) => (
                <Select.Option key={g.id} value={g.id}>
                  {g.id} · {g.host}
                </Select.Option>
              ))}
            </Select>
          </FormItem>
          <FormItem label="成本" field="cost" required rules={[{ required: true, message: '请输入成本' }]}>
            <InputNumber placeholder="0.0100" min={0} step={0.0001} precision={4} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="生效起始" field="time_start" extra="HH:MM，留空表示不限时">
            <Input placeholder="如 09:00" />
          </FormItem>
          <FormItem label="生效结束" field="time_end" extra="与起始同时填写才生效">
            <Input placeholder="如 18:00" />
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
