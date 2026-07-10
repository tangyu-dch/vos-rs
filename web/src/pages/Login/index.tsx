import { useState } from 'react';
import { Button, Card, Form, Input, Message } from '@arco-design/web-react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '@/auth/AuthContext';

const FormItem = Form.Item;

export default function Login() {
  const [loading, setLoading] = useState(false);
  const [form] = Form.useForm();
  const navigate = useNavigate();
  const { login } = useAuth();

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      setLoading(true);
      await login(values.username, values.password);
      navigate('/dashboard', { replace: true });
    } catch (error) {
      if (error instanceof Error && error.message !== '表单校验失败') {
        Message.error('登录失败，请检查用户名和密码');
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="login-page">
      <Card className="login-card" bordered={false}>
        <div className="login-brand">VOS-RS</div>
        <div className="login-subtitle">VoIP 运营管理平台</div>
        <Form form={form} layout="vertical" onSubmit={handleSubmit}>
          <FormItem field="username" label="用户名" rules={[{ required: true, message: '请输入用户名' }]}>
            <Input placeholder="请输入用户名" autoComplete="username" />
          </FormItem>
          <FormItem field="password" label="密码" rules={[{ required: true, message: '请输入密码' }]}>
            <Input.Password placeholder="请输入密码" autoComplete="current-password" />
          </FormItem>
          <Button type="primary" long htmlType="submit" loading={loading}>
            登录
          </Button>
        </Form>
      </Card>
    </div>
  );
}
