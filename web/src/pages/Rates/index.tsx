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
  Message,
  Popconfirm,
  Alert,
  Empty,
} from '@arco-design/web-react';
import { IconPlus, IconEdit, IconDelete, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { BillingRate } from '@/types';

const FormItem = Form.Item;

export default function Rates() {
  const [rates, setRates] = useState<BillingRate[]>([]);
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [modalVisible, setModalVisible] = useState(false);
  const [editing, setEditing] = useState<BillingRate | null>(null);
  const [form] = Form.useForm();

  const load = useCallback(async (nextPage = 1, nextPageSize = 20) => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiService.getRates(nextPage, nextPageSize);
      setRates(data.items);
      setTotal(data.total);
      setPage(nextPage);
      setPageSize(nextPageSize);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取费率列表失败');
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

  const handleEdit = (r: BillingRate) => {
    setEditing(r);
    form.setFieldsValue(r);
    setModalVisible(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await apiService.deleteRate(id);
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
        await apiService.updateRate(editing.id, values);
        Message.success('更新成功');
      } else {
        await apiService.createRate(values);
        Message.success('创建成功');
      }
      setModalVisible(false);
      load();
    } catch {
      /* 校验失败 */
    }
  };

  const columns = [
    {
      title: '费率 ID',
      dataIndex: 'id',
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
    },
    {
      title: '被叫前缀',
      dataIndex: 'prefix',
      width: 140,
      render: (v: string) => <span className="prefix-tag">{v || '*'}</span>,
    },
    {
      title: '费率',
      dataIndex: 'rate_per_minute',
      width: 140,
      render: (v: number) => <span className="cell-mono">¥{v.toFixed(4)}/分钟</span>,
    },
    {
      title: '说明',
      dataIndex: 'description',
      render: (v: string) => v || '—',
    },
    {
      title: '操作',
      dataIndex: 'actions',
      width: 180,
      fixed: 'right' as const,
      render: (_: any, record: BillingRate) => (
        <Space size={4}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm title="确认删除该费率？" icon={null} onOk={() => handleDelete(record.id)}>
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
          <h1>费率</h1>
          <span className="sub">按被叫前缀配置计费费率（最长前缀优先匹配）</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={() => load(page, pageSize)}>
            刷新
          </Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            新建费率
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={rates}
          rowKey="id"
          loading={loading}
          pagination={{ current: page, pageSize, total, sizeCanChange: true, sizeOptions: [10, 20, 50, 100], onChange: (nextPage) => load(nextPage, pageSize), onPageSizeChange: (nextPageSize) => load(1, nextPageSize) }}
          noDataElement={<Empty description="暂无费率配置" />}
        />
      </Card>

      <Modal
        title={editing ? '编辑费率' : '新建费率'}
        visible={modalVisible}
        onOk={handleSubmit}
        onCancel={() => setModalVisible(false)}
        okText="保存"
        cancelText="取消"
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem label="费率 ID" field="id" required rules={[{ required: true, message: '请输入费率 ID' }]}>
            <Input placeholder="如 rate-cn-mobile" disabled={!!editing} />
          </FormItem>
          <FormItem label="被叫前缀" field="prefix" required rules={[{ required: true, message: '请输入前缀' }]}>
            <Input placeholder="如 138（留空匹配所有）" />
          </FormItem>
          <FormItem label="费率" field="rate_per_minute" required rules={[{ required: true, message: '请输入费率' }]}>
            <InputNumber placeholder="0.1000" min={0} step={0.0001} precision={4} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="说明" field="description">
            <Input placeholder="可选" />
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
