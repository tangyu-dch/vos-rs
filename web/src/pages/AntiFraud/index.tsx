import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Button,
  Space,
  Modal,
  Form,
  Input,
  Select,
  Switch,
  Message,
  Popconfirm,
  Alert,
  Empty,
  Tabs,
  Tag,
} from '@arco-design/web-react';
import { IconPlus, IconEdit, IconDelete, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { AntiFraudRule, AntiFraudConfigItem } from '@/types';

const FormItem = Form.Item;

const RULE_TYPE_MAP: Record<string, { color: string; text: string }> = {
  blocked_prefix: { color: 'red', text: '号码黑名单' },
  allowed_prefix: { color: 'green', text: '号码白名单' },
  blocked_ip: { color: 'red', text: 'IP 黑名单' },
  allowed_ip: { color: 'green', text: 'IP 白名单' },
};

export default function AntiFraud() {
  const [rules, setRules] = useState<AntiFraudRule[]>([]);
  const [config, setConfig] = useState<AntiFraudConfigItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modalVisible, setModalVisible] = useState(false);
  const [editingRule, setEditingRule] = useState<AntiFraudRule | null>(null);
  const [form] = Form.useForm();
  const [activeTab, setActiveTab] = useState('rules');

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [r, c] = await Promise.all([
        apiService.getAntiFraudRules(),
        apiService.getAntiFraudConfig(),
      ]);
      setRules(r);
      setConfig(c);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取防盗打配置失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleAdd = () => {
    setEditingRule(null);
    form.resetFields();
    setModalVisible(true);
  };

  const handleEdit = (rule: AntiFraudRule) => {
    setEditingRule(rule);
    form.setFieldsValue(rule);
    setModalVisible(true);
  };

  const handleDelete = async (id: number) => {
    try {
      await apiService.deleteAntiFraudRule(id);
      Message.success('删除成功');
      loadData();
    } catch {
      Message.error('删除失败');
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      if (editingRule) {
        await apiService.updateAntiFraudRule(editingRule.id, {
          description: values.description,
          enabled: values.enabled,
        });
        Message.success('更新成功');
      } else {
        await apiService.createAntiFraudRule({
          rule_type: values.rule_type,
          value: values.value,
          description: values.description,
        });
        Message.success('创建成功');
      }
      setModalVisible(false);
      loadData();
    } catch {
      /* 校验失败 */
    }
  };

  const ruleColumns = [
    {
      title: '类型',
      dataIndex: 'rule_type',
      width: 120,
      render: (v: string) => {
        const m = RULE_TYPE_MAP[v] || { color: 'gray', text: v };
        return <Tag color={m.color}>{m.text}</Tag>;
      },
    },
    {
      title: '值',
      dataIndex: 'value',
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
    },
    {
      title: '说明',
      dataIndex: 'description',
      render: (v: string) => v || '—',
    },
    {
      title: '状态',
      dataIndex: 'enabled',
      width: 90,
      render: (v: boolean) =>
        v ? <Tag color="green">启用</Tag> : <Tag color="gray">禁用</Tag>,
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      width: 170,
      render: (d: string) => (d ? new Date(d).toLocaleString('zh-CN') : '—'),
    },
    {
      title: '操作',
      dataIndex: 'actions',
      width: 150,
      fixed: 'right' as const,
      render: (_: any, record: AntiFraudRule) => (
        <Space size={4}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm title="确认删除该规则？" icon={null} onOk={() => handleDelete(record.id)}>
            <Button type="text" size="small" status="danger" icon={<IconDelete />}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  const configColumns = [
    {
      title: '配置项',
      dataIndex: 'config_key',
      width: 250,
      render: (v: string) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '值',
      dataIndex: 'config_value',
      width: 200,
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
    },
    {
      title: '说明',
      dataIndex: 'description',
      render: (v: string) => v || '—',
    },
    {
      title: '更新时间',
      dataIndex: 'updated_at',
      width: 170,
      render: (d: string) => (d ? new Date(d).toLocaleString('zh-CN') : '—'),
    },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>防盗打管理</h1>
          <span className="sub">配置号码/IP 黑白名单、并发限制、短通话检测等防盗打规则</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={loadData}>
            刷新
          </Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            新建规则
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Tabs activeTab={activeTab} onChange={setActiveTab}>
        <Tabs.TabPane key="rules" title="号码/IP 规则">
          <Card className="app-card" bordered={false} style={{ marginTop: 16 }}>
            <Table
              className="app-table"
              columns={ruleColumns}
              data={rules}
              rowKey="id"
              loading={loading}
              pagination={{ pageSize: 20, sizeCanChange: true, showTotal: true }}
              scroll={{ x: 900 }}
              noDataElement={<Empty description="暂无防盗打规则" />}
            />
          </Card>
        </Tabs.TabPane>

        <Tabs.TabPane key="config" title="全局配置">
          <Card className="app-card" bordered={false} style={{ marginTop: 16 }}>
            <Table
              className="app-table"
              columns={configColumns}
              data={config}
              rowKey="id"
              loading={loading}
              pagination={false}
              noDataElement={<Empty description="暂无配置" />}
            />
          </Card>
        </Tabs.TabPane>
      </Tabs>

      <Modal
        title={editingRule ? '编辑规则' : '新建规则'}
        visible={modalVisible}
        onOk={handleSubmit}
        onCancel={() => setModalVisible(false)}
        okText="保存"
        cancelText="取消"
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem
            label="规则类型"
            field="rule_type"
            required
            rules={[{ required: true, message: '请选择规则类型' }]}
          >
            <Select placeholder="请选择" disabled={!!editingRule}>
              <Select.Option value="blocked_prefix">号码黑名单</Select.Option>
              <Select.Option value="allowed_prefix">号码白名单</Select.Option>
              <Select.Option value="blocked_ip">IP 黑名单</Select.Option>
              <Select.Option value="allowed_ip">IP 白名单</Select.Option>
            </Select>
          </FormItem>
          <FormItem
            label="值"
            field="value"
            required
            rules={[{ required: true, message: '请输入值' }]}
          >
            <Input
              placeholder="号码前缀（如 116）或 IP/CIDR（如 10.0.0.0/24）"
              disabled={!!editingRule}
            />
          </FormItem>
          <FormItem label="说明" field="description">
            <Input placeholder="可选" />
          </FormItem>
          {editingRule && (
            <FormItem label="启用" field="enabled" triggerPropName="checked">
              <Switch />
            </FormItem>
          )}
        </Form>
      </Modal>
    </div>
  );
}
