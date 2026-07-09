import { useState, useEffect, useCallback } from 'react';
import {
  Card, Table, Button, Space, Modal, Form, Input, InputNumber, Select,
  Message, Popconfirm, Alert, Empty, Tag,
} from '@arco-design/web-react';
import { IconPlus, IconEdit, IconDelete, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { SipGateway } from '@/types';

const FormItem = Form.Item;

const TRANSPORT_COLOR: Record<string, string> = {
  udp: '#165dff', tcp: '#0fc6c2', tls: '#722ed1',
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
    try { setGateways(await apiService.getGateways()); }
    catch (err) { setError(err instanceof Error ? err.message : '加载失败'); Message.error('获取网关列表失败'); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => { loadGateways(); }, [loadGateways]);

  const handleAdd = () => {
    setEditingGateway(null);
    form.resetFields();
    form.setFieldsValue({ gateway_type: 'gateway', transport: 'udp', caller_id_mode: 'passthrough', enabled: true });
    setModalVisible(true);
  };

  const handleEdit = (gw: SipGateway) => {
    setEditingGateway(gw);
    form.setFieldsValue(gw);
    setModalVisible(true);
  };

  const handleDelete = async (id: string) => {
    try { await apiService.deleteGateway(id); Message.success('删除成功'); loadGateways(); }
    catch { Message.error('删除失败'); }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      const payload = {
        host: values.host, port: values.port, transport: values.transport,
        max_capacity: values.max_capacity, gateway_type: 'gateway' as const,
        prefix_rules: values.prefix_rules || '', caller_id_mode: values.caller_id_mode || 'passthrough',
        virtual_caller: values.virtual_caller || '', max_concurrent: values.max_concurrent,
        account_id: values.account_id, enabled: values.enabled !== false,
      };
      if (editingGateway) { await apiService.updateGateway(editingGateway.id, payload); Message.success('更新成功'); }
      else { await apiService.createGateway({ id: values.id, ...payload }); Message.success('创建成功'); }
      setModalVisible(false); loadGateways();
    } catch { /* 校验失败 */ }
  };

  const columns = [
    { title: '网关 ID', dataIndex: 'id', render: (v: string) => <span className="cell-mono cell-strong">{v}</span> },
    { title: '主机地址', dataIndex: 'host', render: (v: string) => <span className="cell-mono">{v}</span> },
    { title: '端口', dataIndex: 'port', width: 80, render: (v: number) => <span className="cell-mono">{v || '—'}</span> },
    { title: '协议', dataIndex: 'transport', width: 80, render: (v: string) => <span className="transport-tag" style={{ color: TRANSPORT_COLOR[v] }}>{v?.toUpperCase()}</span> },
    { title: '对接方式', dataIndex: 'caller_id_mode', width: 100, render: (v: string) => v === 'virtual' ? <Tag color="blue">虚拟主叫</Tag> : v === 'random' ? <Tag color="green">随机选号</Tag> : <Tag>透传</Tag> },
    { title: '最大并发', dataIndex: 'max_concurrent', width: 90, render: (v: number) => v ? <span className="cell-mono">{v}</span> : '—' },
    { title: '当前并发', dataIndex: 'current_concurrent', width: 90, render: (v: number) => <span className="cell-mono">{v || 0}</span> },
    { title: '状态', dataIndex: 'enabled', width: 70, render: (v: boolean) => v !== false ? <Tag color="green">启用</Tag> : <Tag color="red">禁用</Tag> },
    { title: '操作', dataIndex: 'actions', width: 140, fixed: 'right' as const,
      render: (_: any, record: SipGateway) => (
        <Space size={4}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>编辑</Button>
          <Popconfirm title="确认删除？" icon={null} onOk={() => handleDelete(record.id)}>
            <Button type="text" size="small" status="danger" icon={<IconDelete />}>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  const callerIdMode = Form.useWatch('caller_id_mode', form);

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title"><h1>落地网关</h1><span className="sub">配置出局中继网关（我们主动拨出到对端）</span></div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={loadGateways}>刷新</Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>新建网关</Button>
        </div>
      </div>
      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}
      <Card className="app-card" bordered={false}>
        <Table className="app-table" columns={columns} data={gateways.filter(g => g.gateway_type === 'gateway')} rowKey="id" loading={loading}
          pagination={{ pageSize: 20, sizeCanChange: true, sizeOptions: [10, 20, 50, 100] }} scroll={{ x: 1200 }}
          noDataElement={<Empty description="暂无落地网关" />} />
      </Card>
      <Modal title={editingGateway ? '编辑落地网关' : '新建落地网关'} visible={modalVisible} onOk={handleSubmit} onCancel={() => setModalVisible(false)} okText="保存" cancelText="取消">
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem label="网关 ID" field="id" required rules={[{ required: true, message: '请输入' }]}>
            <Input placeholder="如 gw1" disabled={!!editingGateway} />
          </FormItem>
          <FormItem label="主机地址" field="host" required rules={[{ required: true, message: '请输入' }]}>
            <Input placeholder="如 sip.example.com" />
          </FormItem>
          <FormItem label="端口" field="port">
            <InputNumber placeholder="5060" min={1} max={65535} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="传输协议" field="transport" required>
            <Select><Select.Option value="udp">UDP</Select.Option><Select.Option value="tcp">TCP</Select.Option><Select.Option value="tls">TLS</Select.Option></Select>
          </FormItem>
          <FormItem label="对接方式" field="caller_id_mode">
            <Select defaultValue="passthrough">
              <Select.Option value="passthrough">透传（原号码）</Select.Option>
              <Select.Option value="virtual">虚拟主叫</Select.Option>
              <Select.Option value="random">随机选号</Select.Option>
            </Select>
          </FormItem>
          {callerIdMode === 'virtual' && (
            <FormItem label="虚拟主叫" field="virtual_caller" extra="当对接方式选择虚拟主叫时，使用此号码作为 From 主叫">
              <Input placeholder="如 4001001" />
            </FormItem>
          )}
          <FormItem label="最大并发" field="max_concurrent">
            <InputNumber placeholder="100" min={1} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="前缀规则" field="prefix_rules" extra="格式: abc:def(替换) :def(添加) abc:(剥离)，逗号分隔多条">
            <Input.TextArea placeholder="示例：sdf:abc,:86,00:" rows={2} />
          </FormItem>
          <FormItem label="启用" field="enabled">
            <Select defaultValue="true"><Select.Option value="true">启用</Select.Option><Select.Option value="false">禁用</Select.Option></Select>
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
