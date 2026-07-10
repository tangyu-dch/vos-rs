import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Button,
  Space,
  Modal,
  Form,
  Input,
  Message,
  Popconfirm,
  Alert,
  Empty,
} from '@arco-design/web-react';
import { IconPlus, IconEdit, IconDelete, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { SipUser } from '@/types';

const FormItem = Form.Item;

export default function Users() {
  const [users, setUsers] = useState<SipUser[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modalVisible, setModalVisible] = useState(false);
  const [editingUser, setEditingUser] = useState<SipUser | null>(null);
  const [form] = Form.useForm();

  const loadUsers = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiService.getUsers();
      setUsers(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取用户列表失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadUsers();
  }, [loadUsers]);

  const handleAdd = () => {
    setEditingUser(null);
    form.resetFields();
    setModalVisible(true);
  };

  const handleEdit = (user: SipUser) => {
    setEditingUser(user);
    form.resetFields();
    setModalVisible(true);
  };

  const handleDelete = async (username: string) => {
    try {
      await apiService.deleteUser(username);
      Message.success('删除成功');
      loadUsers();
    } catch {
      Message.error('删除失败');
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      if (editingUser) {
        await apiService.updateUser(editingUser.username, values.password);
        Message.success('更新成功');
      } else {
        await apiService.createUser(values);
        Message.success('创建成功');
      }
      setModalVisible(false);
      loadUsers();
    } catch {
      /* 校验失败 */
    }
  };

  const columns = [
    {
      title: '用户名',
      dataIndex: 'username',
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
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
      render: (_: any, record: SipUser) => (
        <Space size={4}>
          <Button type="text" size="small" icon={<IconEdit />} onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm
            title="确认删除该用户？"
            icon={null}
            onOk={() => handleDelete(record.username)}
          >
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
          <h1>SIP 用户</h1>
          <span className="sub">管理可注册的 SIP 账户凭证</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={loadUsers}>
            刷新
          </Button>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            新建用户
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={users}
          rowKey="username"
          loading={loading}
          pagination={{ pageSize: 20, sizeCanChange: true }}
          noDataElement={<Empty description="暂无 SIP 用户" />}
        />
      </Card>

      <Modal
        title={editingUser ? '编辑用户' : '新建用户'}
        visible={modalVisible}
        onOk={handleSubmit}
        onCancel={() => setModalVisible(false)}
        okText="保存"
        cancelText="取消"
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 19 }}>
          <FormItem
            label="用户名"
            field="username"
            required
            rules={[{ required: true, message: '请输入用户名' }]}
          >
            <Input placeholder="如 1001" disabled={!!editingUser} />
          </FormItem>
          <FormItem
            label="密码"
            field="password"
            required
            rules={[{ required: true, message: '请输入密码' }]}
          >
            <Input.Password placeholder="SIP Digest 认证密码" />
          </FormItem>
        </Form>
      </Modal>
    </div>
  );
}
