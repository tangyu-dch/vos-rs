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
import type { SipGateway } from '@/types';

const FormItem = Form.Item;

const TRANSPORT_MAP: Record<string, string> = { udp: '#6366f1', tcp: '#06b6d4', tls: '#8b5cf6' };

export default function PeerGateways() {
  const [gateways, setGateways] = useState<SipGateway[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modalVisible, setModalVisible] = useState(false);
  const [editingGateway, setEditingGateway] = useState<SipGateway | null>(null);
  const [form] = Form.useForm();

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setGateways(await apiService.getGateways());
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { loadData(); }, [loadData]);

  const handleAdd = () => {
    setEditingGateway(null);
    form.resetFields();
    form.setFieldsValue({ gateway_type: 'peer', transport: 'udp', supports_registration: false });
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
      loadData();
    } catch { Message.error('删除失败'); }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      const payload = {
        host: values.host,
        port: values.port,
        transport: values.transport,
        max_capacity: values.max_capacity,
        gateway_type: 'peer' as const,
        prefix_rules: values.prefix_rules || '',
        supports_registration: values.supports_registration || false,
        reg_auth_type: values.reg_auth_type || 'ip',
        reg_username: values.reg_username || '',
        caller_id_mode: values.caller_id_mode || 'passthrough',
        virtual_caller: values.virtual_caller || '',
        enabled: true,
      };
      if (editingGateway) {
        await apiService.updateGateway(editingGateway.id, payload);
        Message.success('更新成功');
      } else {
        await apiService.createGateway({ id: values.id, ...payload });
        Message.success('创建成功');
      }
      setModalVisible(false);
      loadData();
    } catch { /* 校验失败 */ }
  };

  const columns = [
    { title: 'ID', dataIndex: 'id', width: 160, fixed: 'left' as const, render: (v: string) => <span className="cell-mono cell-strong">{v}</span> },
    { title: '主机地址', dataIndex: 'host', minWidth: 180, render: (v: string) => <span className="cell-mono">{v}</span> },
    { title: '端口', dataIndex: 'port', width: 100, render: (v: number) => <span className="cell-mono">{v || '—'}</span> },
    { title: '协议', dataIndex: 'transport', width: 100, render: (v: string) => <Tag color={TRANSPORT_MAP[v] || '#94a3b8'} style={{ borderRadius: 6 }}>{v?.toUpperCase()}</Tag> },
    { title: '最大容量', dataIndex: 'max_capacity', width: 120, render: (v: number) => v ? <span className="cell-mono">{v}</span> : '—' },
    {
      title: '前缀规则', dataIndex: 'prefix_rules', minWidth: 300,
      render: (v: string) => {
        if (!v) return '—';
        return (
          <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
            {v.split(',').map((rule, i) => {
              const r = rule.trim();
              const [from, to] = r.split(':');
              let color = '#6366f1';
              let display = r;
              if (from && to) { color = '#8b5cf6'; display = `${from} → ${to}`; }
              else if (!from && to) { color = '#10b981'; display = `+${to}`; }
              else if (from && !to) { color = '#ef4444'; display = `-${from}`; }
              return <Tag key={i} color={color} style={{ borderRadius: 4, fontFamily: 'monospace', fontSize: 11 }}>{display}</Tag>;
            })}
          </div>
        );
      },
    },
    { title: '创建时间', dataIndex: 'created_at', width: 190, render: (d: string) => d ? new Date(d).toLocaleString('zh-CN') : '—' },
    {
      title: '操作', dataIndex: 'actions', width: 180, fixed: 'right' as const,
      render: (_: any, record: SipGateway) => (
        <Space size={8}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>编辑</Button>
          <Popconfirm title="确认删除该网关？" icon={null} onOk={() => handleDelete(record.id)}>
            <Button type="text" size="small" status="danger" icon={<IconDelete />}>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  const peerList = gateways.filter((g) => g.gateway_type === 'peer');

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>对接网关</h1>
          <span className="sub">我们主动连接其他运营商线路，支持前缀规则处理</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={loadData}>刷新</Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>新建对接网关</Button>
        </div>
      </div>

      <div style={{ padding: '12px 16px', background: '#eff6ff', borderRadius: 8, marginBottom: 16, display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{ fontSize: 16 }}>🔗</span>
        <span style={{ color: '#2563eb', fontWeight: 500 }}>对接网关 — 我们主动连向其他运营商线路，支持前缀规则处理</span>
      </div>

      <div style={{ padding: '10px 16px', background: '#f8fafc', borderRadius: 8, marginBottom: 16, border: '1px solid #e2e8f0' }}>
        <span style={{ color: '#64748b', fontSize: 13 }}>
          <strong>前缀规则格式：</strong>
          <code style={{ background: '#e2e8f0', padding: '1px 6px', borderRadius: 4 }}>abc:def</code> 替换 ·
          <code style={{ background: '#e2e8f0', padding: '1px 6px', borderRadius: 4, marginLeft: 4 }}>:def</code> 添加 ·
          <code style={{ background: '#e2e8f0', padding: '1px 6px', borderRadius: 4, marginLeft: 4 }}>abc:</code> 剥离 ·
          多条规则用英文逗号分隔
        </span>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false}>
        <Table className="app-table app-table--fixed" columns={columns} data={peerList} rowKey="id" loading={loading} pagination={{ pageSize: 20, sizeCanChange: true, sizeOptions: [10, 20, 50, 100] }} scroll={{ x: 1600 }} noDataElement={<Empty description="暂无对接网关" />} />
      </Card>

      <Modal title={editingGateway ? '编辑对接网关' : '新建对接网关'} visible={modalVisible} onOk={handleSubmit} onCancel={() => setModalVisible(false)} okText="保存" cancelText="取消">
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem label="网关 ID" field="id" required rules={[{ required: true, message: '请输入' }]}>
            <Input placeholder="如 peer-cm" disabled={!!editingGateway} />
          </FormItem>
          <FormItem label="主机地址" field="host" required rules={[{ required: true, message: '请输入' }]}>
            <Input placeholder="如 sip.carrier.com" />
          </FormItem>
          <FormItem label="端口" field="port">
            <InputNumber placeholder="5060" min={1} max={65535} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="传输协议" field="transport" required rules={[{ required: true, message: '请选择' }]}>
            <Select><Select.Option value="udp">UDP</Select.Option><Select.Option value="tcp">TCP</Select.Option><Select.Option value="tls">TLS</Select.Option></Select>
          </FormItem>
          <FormItem label="最大容量" field="max_capacity">
            <InputNumber placeholder="不限" min={1} style={{ width: '100%' }} />
          </FormItem>
          <FormItem label="注册支持" field="supports_registration">
            <Select defaultValue="false"><Select.Option value="true">开启</Select.Option><Select.Option value="false">关闭</Select.Option></Select>
          </FormItem>
          <FormItem label="对接方式" field="reg_auth_type">
            <Select defaultValue="ip">
              <Select.Option value="ip">IP 认证（基于来源 IP）</Select.Option>
              <Select.Option value="digest">Digest 认证（用户名密码）</Select.Option>
            </Select>
          </FormItem>
          {form.getFieldValue('reg_auth_type') === 'digest' && (
            <>
              <FormItem label="用户名" field="reg_username" required rules={[{ required: true, message: '请输入' }]}>
                <Input placeholder="SIP Digest 用户名" />
              </FormItem>
              <FormItem label="密码" field="reg_password">
                <Input.Password placeholder="SIP Digest 密码" />
              </FormItem>
            </>
          )}
          <FormItem label="前缀规则" field="prefix_rules" extra={<span style={{ fontSize: 12, color: '#94a3b8' }}>格式：<code>abc:def</code> 替换 · <code>:def</code> 添加 · <code>abc:</code> 剥离 · 逗号分隔多条</span>}>
            <Input.TextArea placeholder="示例：sdf:abc,:86,00:" rows={2} />
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
