import { useState } from 'react';
import { Alert, Button, Form, Input } from '@arco-design/web-react';
import { IconCheckCircle, IconLock, IconSafe, IconUser } from '@arco-design/web-react/icon';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '../../auth/AuthContext';

export default function Login() {
  const { session, login } = useAuth();
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();
  if (session) return <Navigate to="/overview" replace />;
  const submit = async (values: { username: string; password: string }) => {
    setLoading(true); setError('');
    try { await login(values.username, values.password); navigate((location.state as { from?: string } | null)?.from || '/overview', { replace: true }); }
    catch (reason) { setError(reason instanceof Error ? reason.message : '登录失败'); }
    finally { setLoading(false); }
  };
  return <main className="login-screen">
    <aside className="login-status" aria-label="系统状态">
      <div className="login-status-brand"><span className="brand-mark">V</span><div><strong>VOS Console</strong><small>SOFTSWITCH CONTROL</small></div></div>
      <div className="login-status-copy"><span>VOS-RS / CONTROL PLANE</span><h2>实时掌控每一条<br />通信链路</h2><p>统一的信令、媒体、路由与计费控制面。</p></div>
      <div className="login-health"><IconCheckCircle /><div><strong>控制面运行正常</strong><span>Control plane operational</span></div></div>
    </aside>
    <section className="login-panel">
      <div className="login-form-wrap">
        <div className="login-brand"><span className="brand-mark">V</span><div><strong>VOS Console</strong><small>统一通信运行控制台</small></div></div>
        <div className="login-copy"><span className="login-eyebrow"><IconSafe />安全访问</span><h1>欢迎回来</h1><p>请使用控制台账户登录</p></div>
        {error && <Alert type="error" content={error} closable onClose={() => setError('')} />}
        <Form className="login-form" layout="vertical" onSubmit={submit} autoComplete="on">
          <Form.Item label="用户名" field="username" rules={[{ required: true, message: '请输入用户名' }]}><Input prefix={<IconUser />} size="large" placeholder="请输入用户名" autoComplete="username" /></Form.Item>
          <Form.Item label="密码" field="password" rules={[{ required: true, message: '请输入密码' }]}><Input.Password prefix={<IconLock />} size="large" placeholder="请输入密码" autoComplete="current-password" /></Form.Item>
          <Button className="login-submit" type="primary" htmlType="submit" size="large" loading={loading} long>登录控制台</Button>
        </Form>
        <div className="login-foot"><IconLock />账户访问受角色权限与审计策略保护</div>
      </div>
    </section>
  </main>;
}
