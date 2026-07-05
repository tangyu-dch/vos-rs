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
import type { SipGateway } from '@/types';

const FormItem = Form.Item;

const TRANSPORT_COLOR: Record<string, string> = {
  udp: '#165dff',
  tcp: '#0fc6c2',
  tls: '#722ed1',
};

export default function Gateways() {
  const [gateways, setGateways] = useState<SipGateway[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modalVisible, setModalVisible] = useState(false);
  const [editingGateway, setEditingGateway] = useState<SipGateway | null>(null);
  const [form] = Form.useForm();

  const loadGateways = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiService.getGateways();
      setGateways(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取网关列表失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadGateways();
  }, [loadGateways]);

  const handleAdd = () => {
    setEditingGateway(null);
    form.resetFields();
    setModalVisible(true);
  };

  const handleEdit = (gw: SipGateway) => {
    setEditingGateway(gw);
    form.setFieldsValue(gw);
    setModalVisible(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await apiService.deleteGateway(id);
      Message.success('删除成功');
      loadGateways();
    } catch {
      Message.error('删除失败');
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      if (editingGateway) {
        await apiService.updateGateway(editingGateway.id, values);
        Message.success('更新成功');
      } else {
        await apiService.createGateway(values);
        Message.success('创建成功');
      }
      setModalVisible(false);
      loadGateways();
    } catch {
      /* 校验失败 */
    }
  };

  const columns = [
    {
      title: '网关 ID',
      dataIndex: 'id',
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
    },
    {
      title: '主机地址',
      dataIndex: 'host',
      render: (v: string) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '端口',
      dataIndex: 'port',
      width: 90,
      render: (v: number) => <span className="cell-mono">{v || '—'}</span>,
    },
    {
      title: '传输协议',
      dataIndex: 'transport',
      width: 110,
      render: (v: string) => (
        <span
          className="transport-tag"
          style={{ color: TRANSPORT_COLOR[v], background: `${TRANSPORT_COLOR[v]}1a` }}
        >
          {v.toUpperCase()}
        </span>
      ),
    },
    {
      title: '最大容量',
      dataIndex: 'max_capacity',
      width: 100,
      render: (v: number) => (v ? <span className="cell-mono">{v}</span> : '—'),
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
      render: (_: any, record: SipGateway) => (
        <Space size={4}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm title="确认删除该网关？" icon={null} onOk={() => handleDelete(record.id)}>
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
          <h1>网关管理</h1>
          <span className="sub">配置出局中继网关及容量</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={loadGateways}>
            刷新
          </Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            新建网关
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={gateways}
          rowKey="id"
          loading={loading}
          pagination={{ pageSize: 20, sizeCanChange: true }}
          scroll={{ x: 960 }}
          noDataElement={<Empty description="暂无网关" />}
        />
      </Card>

      <Modal
        title={editingGateway ? '编辑网关' : '新建网关'}
        visible={modalVisible}
        onOk={handleSubmit}
        onCancel={() => setModalVisible(false)}
        okText="保存"
        cancelText="取消"
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem label="网关 ID" field="id" required rules={[{ required: true, message: '请输入网关 ID' }]}>
            <Input placeholder="如 gw1" disabled={!!editingGateway} />
          </FormItem>
          <FormItem label="主机地址" field="host" required rules={[{ required: true, message: '请输入主机地址' }]}>
            <Input placeholder="如 sip.example.com" />
          </FormItem>
          <FormItem label="端口" field="port">
            <InputNumber placeholder="5060" min={1} max={65535} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="传输协议" field="transport" required rules={[{ required: true, message: '请选择传输协议' }]}>
            <Select placeholder="请选择">
              <Select.Option value="udp">UDP</Select.Option>
              <Select.Option value="tcp">TCP</Select.Option>
              <Select.Option value="tls">TLS</Select.Option>
            </Select>
          </FormItem>
          <FormItem label="最大容量" field="max_capacity">
            <InputNumber placeholder="100" min={1} style={{ width: '100%' }} />
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
