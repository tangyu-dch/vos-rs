import { useState, type FormEvent } from 'react';
import { Card, CardBody, Input, Button, Chip, Tooltip } from '@heroui/react';
import { User, Lock, ShieldCheck, ArrowRight, Sparkles, Sun, Moon } from 'lucide-react';
import { Navigate, useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '@/auth/AuthContext';
import { useTheme } from '@/theme/ThemeContext';
import { canAccessPage, firstMenuPath } from '@/services/auth';

export default function Login() {
  const { session, login } = useAuth();
  const { theme, toggleTheme } = useTheme();
  const [username, setUsername] = useState('admin');
  const [password, setPassword] = useState('admin');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();

  if (session) return <Navigate to={firstMenuPath(session)} replace />;

  const handleSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!username || !password) {
      setError('请输入用户名和密码');
      return;
    }
    setLoading(true);
    setError('');
    try {
      const next = await login(username, password);
      const requestedPath = (location.state as { from?: string } | null)?.from;
      navigate(
        requestedPath && canAccessPage(next, requestedPath) ? requestedPath : firstMenuPath(next),
        { replace: true },
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '登录失败');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="relative min-h-screen grid grid-cols-1 lg:grid-cols-12 bg-content1">
      {/* 右上角主题切换按钮 (固定定位, 全屏可见) */}
      <div className="fixed top-4 right-4 z-50">
        <Tooltip
          content={theme === 'dark' ? '切换到亮色主题' : '切换到暗色主题'}
          placement="bottom-end"
          delay={200}
        >
          <Button
            isIconOnly
            size="sm"
            variant="flat"
            onPress={toggleTheme}
            aria-label={theme === 'dark' ? '切换到亮色主题' : '切换到暗色主题'}
            className="bg-content1/80 backdrop-blur-md border border-default-200 hover:bg-content2"
          >
            {theme === 'dark' ? (
              <Sun className="w-4 h-4 text-warning" />
            ) : (
              <Moon className="w-4 h-4 text-primary" />
            )}
          </Button>
        </Tooltip>
      </div>

      {/* 左侧：品牌介绍区 */}
      <div className="relative hidden lg:flex lg:col-span-7 flex-col justify-between p-16 bg-content2 overflow-hidden">
        <div className="relative z-10 flex items-center gap-3">
          <div className="w-10 h-10 rounded-lg bg-primary flex items-center justify-center font-semibold text-lg text-primary-foreground">
            V
          </div>
          <div>
            <h2 className="font-semibold text-large tracking-tight text-foreground leading-tight">
              话务平台
            </h2>
            <p className="text-tiny text-default-400 tracking-wide">软交换控制台</p>
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
            电信级话务控制平台
          </Chip>
          <h1 className="text-4xl font-semibold tracking-tight leading-tight mb-6 text-foreground">
            电信级软交换控制平台
          </h1>
          <p className="text-default-500 text-medium leading-relaxed mb-8">
            面向高并发话务场景，统一提供信令、媒体、路由与计费的实时监控和运营管理能力。
          </p>

          <div className="grid grid-cols-2 gap-4">
            <Card>
              <CardBody className="p-4">
                <div className="text-xl font-semibold text-foreground tnum">1,700+</div>
                <div className="text-tiny text-default-500 mt-1">目标并发通话</div>
              </CardBody>
            </Card>
            <Card>
              <CardBody className="p-4">
                <div className="text-xl font-semibold text-foreground tnum">小于 1 毫秒</div>
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
                  <div className="text-small font-semibold text-foreground">核心引擎已就绪</div>
                  <div className="text-tiny text-default-500">控制面运行正常</div>
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
              <h2 className="text-xl font-semibold text-foreground tracking-tight">
                欢迎登录控制台
              </h2>
              <p className="text-tiny text-default-500 mt-1.5">输入账户凭据进入话务管理平台</p>
            </div>

            {error && (
              <Card className="border border-danger/30 bg-danger/10">
                <CardBody className="text-tiny text-danger font-medium p-3">{error}</CardBody>
              </Card>
            )}

            <form onSubmit={handleSubmit} className="flex flex-col gap-5">
              <Input
                label="控制台账号"
                placeholder="请输入用户名，例如 admin"
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
                placeholder="请输入密码"
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
