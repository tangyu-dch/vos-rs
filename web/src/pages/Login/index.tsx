import { useState } from 'react';
import { Button, Form, Input, Message } from '@arco-design/web-react';
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
      <div className="login-container">
        {/* 左侧品牌区 */}
        <div className="login-brand-section">
          <div className="login-brand-content">
            <div className="login-logo">
              <svg width="48" height="48" viewBox="0 0 48 48" fill="none">
                <rect width="48" height="48" rx="12" fill="var(--accent)" fillOpacity="0.15"/>
                <path d="M16 20C16 17.7909 17.7909 16 20 16H28C30.2091 16 32 17.7909 32 20V28C32 30.2091 30.2091 32 28 32H20C17.7909 32 16 30.2091 16 28V20Z" stroke="var(--accent)" strokeWidth="2"/>
                <circle cx="24" cy="24" r="3" fill="var(--accent)"/>
                <path d="M24 16V20M24 28V32M16 24H20M28 24H32" stroke="var(--accent)" strokeWidth="1.5" strokeLinecap="round"/>
              </svg>
            </div>
            <h1 className="login-title">VOS-RS</h1>
            <p className="login-desc">VoIP 软交换运营管理平台</p>
            <div className="login-features">
              <div className="login-feature">
                <span className="feature-icon">⚡</span>
                <span>高性能信令处理</span>
              </div>
              <div className="login-feature">
                <span className="feature-icon"> ️</span>
                <span>全方位安全防护</span>
              </div>
              <div className="login-feature">
                <span className="feature-icon"> </span>
                <span>实时呼叫监控</span>
              </div>
              <div className="login-feature">
                <span className="feature-icon"> </span>
                <span>智能路由引擎</span>
              </div>
            </div>
          </div>
        </div>

        {/* 右侧登录区 */}
        <div className="login-form-section">
          <div className="login-form-wrapper">
            <div className="login-form-header">
              <h2>欢迎回来</h2>
              <p>请登录您的账户</p>
            </div>
            <Form form={form} layout="vertical" onSubmit={handleSubmit} className="login-form">
              <FormItem field="username" label="用户名" rules={[{ required: true, message: '请输入用户名' }]}>
                <Input placeholder="请输入用户名" autoComplete="username" size="large" />
              </FormItem>
              <FormItem field="password" label="密码" rules={[{ required: true, message: '请输入密码' }]}>
                <Input.Password placeholder="请输入密码" autoComplete="current-password" size="large" />
              </FormItem>
              <Button type="primary" long htmlType="submit" loading={loading} size="large" className="login-btn">
                登录
              </Button>
            </Form>
            <div className="login-footer">
              <span className="login-hint">默认账户：admin / admin123</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
