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
  Tag,
} from '@arco-design/web-react';
import { IconPlus, IconEdit, IconDelete, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { NumberInventory } from '@/types';

const FormItem = Form.Item;

const STATUS_MAP: Record<string, { color: string; text: string }> = {
  available: { color: 'green', text: '可用' },
  assigned: { color: 'blue', text: '已分配' },
  blocked: { color: 'red', text: '已停用' },
};

export default function Numbers() {
  const [numbers, setNumbers] = useState<NumberInventory[]>([]);
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [modalVisible, setModalVisible] = useState(false);
  const [editing, setEditing] = useState<NumberInventory | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async (nextPage = 1, nextPageSize = 20) => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiService.getNumbers(nextPage, nextPageSize);
      setNumbers(data.items);
      setTotal(data.total);
      setPage(nextPage);
      setPageSize(nextPageSize);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取号码列表失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const handleAdd = () => {
    setEditing(null);
    form.resetFields();
    setModalVisible(true);
  };

  const handleEdit = (n: NumberInventory) => {
    setEditing(n);
    form.setFieldsValue({ username: n.username || '', status: n.status, gateway_id: n.gateway_id || '', direction: n.direction || 'bidirectional', max_concurrent: n.max_concurrent });
    setModalVisible(true);
  };

  const handleDelete = async (number: string) => {
    try {
      await apiService.deleteNumber(number);
      Message.success('删除成功');
      load();
    } catch {
      Message.error('删除失败');
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      if (editing) {
        await apiService.updateNumber(editing.number, {
          username: values.username || undefined,
          gateway_id: values.gateway_id || undefined,
          direction: values.direction || 'bidirectional',
          max_concurrent: values.max_concurrent,
          status: values.status,
        });
        Message.success('更新成功');
      } else {
        await apiService.createNumber({
          number: values.number,
          username: values.username || undefined,
          gateway_id: values.gateway_id || undefined,
          direction: values.direction || 'bidirectional',
          max_concurrent: values.max_concurrent,
          status: values.status || 'available',
        });
        Message.success('创建成功');
      }
      setModalVisible(false);
      load();
    } catch {
      /* 校验失败 */
    }
  };

  const columns = [
    { title: '号码', dataIndex: 'number', render: (v: string) => <span className="cell-mono cell-strong">{v}</span> },
    { title: '归属用户', dataIndex: 'username', render: (v: string) => (v ? <span className="cell-mono">{v}</span> : '—') },
    { title: '归属网关', dataIndex: 'gateway_id', width: 120, render: (v: string) => (v ? <span className="cell-mono">{v}</span> : <span className="cell-dash">—</span>) },
    { title: '方向', dataIndex: 'direction', width: 100, render: (v: string) => {
      const m = v === 'inbound' ? { color: 'blue', text: '呼入' } : v === 'outbound' ? { color: 'green', text: '呼出' } : { color: 'purple', text: '双向' };
      return <Tag color={m.color}>{m.text}</Tag>;
    }},
    { title: '最大并发', dataIndex: 'max_concurrent', width: 90, render: (v: number) => v ? <span className="cell-mono">{v}</span> : '—' },
    { title: '状态', dataIndex: 'status', width: 100,
      render: (s: string) => {
        const m = STATUS_MAP[s] || { color: 'gray', text: s };
        return <Tag color={m.color}>{m.text}</Tag>;
      },
    },
    { title: '创建时间', dataIndex: 'created_at', render: (d: string) => (d ? new Date(d).toLocaleString('zh-CN') : '—') },
    { title: '操作', dataIndex: 'actions', width: 180, fixed: 'right' as const,
      render: (_: any, record: NumberInventory) => (
        <Space size={4}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>编辑</Button>
          <Popconfirm title="确认删除该号码？" icon={null} onOk={() => handleDelete(record.number)}>
            <Button type="text" size="small" status="danger" icon={<IconDelete />}>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>号码库存</h1>
          <span className="sub">管理号码资源与分配状态</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={() => load(page, pageSize)}>
            刷新
          </Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            新增号码
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={numbers}
          rowKey="number"
          loading={loading}
          pagination={{ current: page, pageSize, total, sizeCanChange: true, sizeOptions: [10, 20, 50, 100], onChange: (nextPage) => load(nextPage, pageSize), onPageSizeChange: (nextPageSize) => load(1, nextPageSize) }}
          noDataElement={<Empty description="暂无号码" />}
        />
      </Card>

      <Modal
        title={editing ? '编辑号码' : '新增号码'}
        visible={modalVisible}
        onOk={handleSubmit}
        onCancel={() => setModalVisible(false)}
        okText="保存"
        cancelText="取消"
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} initialValues={{ status: 'available', direction: 'bidirectional', max_concurrent: 10 }}>
          <FormItem label="号码" field="number" required rules={[{ required: true, message: '请输入号码' }]}>
            <Input placeholder="如 13800138000" disabled={!!editing} />
          </FormItem>
          <FormItem label="归属用户" field="username">
            <Input placeholder="可选，如 1001" />
          </FormItem>
          <FormItem label="归属网关" field="gateway_id">
            <Input placeholder="网关 ID，如 gw1" />
          </FormItem>
          <FormItem label="方向" field="direction">
            <Select>
              <Select.Option value="bidirectional">双向</Select.Option>
              <Select.Option value="inbound">呼入</Select.Option>
              <Select.Option value="outbound">呼出</Select.Option>
            </Select>
          </FormItem>
          <FormItem label="最大并发" field="max_concurrent">
            <InputNumber placeholder="10" min={1} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="状态" field="status" required rules={[{ required: true, message: '请选择状态' }]}>
            <Select>
              <Select.Option value="available">可用</Select.Option>
              <Select.Option value="assigned">已分配</Select.Option>
              <Select.Option value="blocked">已停用</Select.Option>
            </Select>
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
