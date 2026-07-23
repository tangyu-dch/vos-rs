import { useState, type FormEvent } from 'react';
import { Card, CardBody, Input, Button, Chip } from '@heroui/react';
import { User, Lock, ShieldCheck, ArrowRight, Sparkles } from 'lucide-react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '@/auth/AuthContext';

export default function Login() {
  const { session, login } = useAuth();
  const [username, setUsername] = useState('admin');
  const [password, setPassword] = useState('admin');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();

  if (session) return <Navigate to="/overview" replace />;

  const handleSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!username || !password) {
      setError('请输入用户名和密码');
      return;
    }
    setLoading(true);
    setError('');
    try {
      await login(username, password);
      navigate((location.state as { from?: string } | null)?.from || '/overview', { replace: true });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '登录失败');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen grid grid-cols-1 lg:grid-cols-12 bg-content1">
      {/* 左侧：品牌介绍区 */}
      <div className="relative hidden lg:flex lg:col-span-7 flex-col justify-between p-16 bg-content2 overflow-hidden">
        <div className="relative z-10 flex items-center gap-3">
          <div className="w-11 h-11 rounded-medium bg-primary flex items-center justify-center font-black text-2xl text-foreground">
            V
          </div>
          <div>
            <h2 className="font-bold text-large tracking-tight text-foreground leading-tight">VOS Console</h2>
            <p className="text-tiny font-medium text-primary tracking-widest uppercase">Softswitch Engine v1.0</p>
          </div>
        </div>

        <div className="relative z-10 max-w-xl my-auto py-12">
          <Chip
            color="primary"
            variant="flat"
            size="sm"
            startContent={<Sparkles className="w-3.5 h-3.5" />}
            className="mb-6"
          >
            NEXT-GEN TELECOM CONTROL PLANE
          </Chip>
          <h1 className="text-5xl font-bold tracking-tight leading-none mb-6 text-foreground">
            电信级软交换
            <br />
            <span className="text-primary">高并发控制面</span>
          </h1>
          <p className="text-default-500 text-medium leading-relaxed mb-8">
            专为高并发 VoIP 软交换打造的 Rust 架构引擎。实时可视化监控 SIP 信令、RTP 媒体流与计费结算，单机支持超千路并发通话。
          </p>

          <div className="grid grid-cols-2 gap-4">
            <Card>
              <CardBody className="p-4">
                <div className="text-2xl font-bold text-primary">1,700+</div>
                <div className="text-tiny text-default-500 mt-1">目标并发通话 (CAPS)</div>
              </CardBody>
            </Card>
            <Card>
              <CardBody className="p-4">
                <div className="text-2xl font-bold text-primary">&lt; 1ms</div>
                <div className="text-tiny text-default-500 mt-1">路由计算耗时</div>
              </CardBody>
            </Card>
          </div>
        </div>

        <div className="relative z-10">
          <Card>
            <CardBody className="flex-row items-center justify-between p-4">
              <div className="flex items-center gap-3">
                <ShieldCheck className="w-5 h-5 text-success" />
                <div>
                  <div className="text-small font-semibold text-foreground">Rust B2BUA 内核已就绪</div>
                  <div className="text-tiny text-default-500">Control plane operational</div>
                </div>
              </div>
              <Chip size="sm" variant="dot" color="success">
                集群在线
              </Chip>
            </CardBody>
          </Card>
        </div>
      </div>

      {/* 右侧：登录表单 */}
      <div className="lg:col-span-5 flex items-center justify-center p-6 sm:p-12 bg-content1">
        <Card className="w-full max-w-md">
          <CardBody className="gap-6 p-8">
            <div>
              <div className="inline-flex items-center gap-2 text-tiny font-semibold text-primary mb-2">
                <ShieldCheck className="w-4 h-4" />
                <span>安全访问通道</span>
              </div>
              <h2 className="text-2xl font-bold text-foreground tracking-tight">欢迎登录控制台</h2>
              <p className="text-tiny text-default-500 mt-1.5">输入您的管理员凭据以接入 VOS 软交换平台</p>
            </div>

            {error && (
              <Card className="border border-danger/30 bg-danger/10">
                <CardBody className="text-tiny text-danger font-medium p-3">
                  {error}
                </CardBody>
              </Card>
            )}

            <form onSubmit={handleSubmit} className="flex flex-col gap-5">
              <Input
                label="控制台账号"
                placeholder="请输入用户名 (如 admin)"
                variant="bordered"
                size="lg"
                startContent={<User className="w-4 h-4 text-default-400" />}
                value={username}
                onValueChange={setUsername}
                isRequired
              />
              <Input
                label="访问密码"
                type="password"
                placeholder="请输入密码 (如 admin)"
                variant="bordered"
                size="lg"
                startContent={<Lock className="w-4 h-4 text-default-400" />}
                value={password}
                onValueChange={setPassword}
                isRequired
              />
              <Button
                type="submit"
                color="primary"
                size="lg"
                className="w-full font-semibold mt-2"
                isLoading={loading}
                endContent={<ArrowRight className="w-5 h-5" />}
              >
                接入控制台
              </Button>
            </form>

            <div className="flex items-center justify-between text-tiny text-default-500 pt-2 border-t border-default-200">
              <div className="flex items-center gap-1.5">
                <Lock className="w-3.5 h-3.5" />
                <span>Role-Based Access Control</span>
              </div>
              <span className="font-mono text-primary">v1.0.0</span>
            </div>
          </CardBody>
        </Card>
      </div>
    </div>
  );
}
