import { Button } from '@heroui/react';
import { ArrowLeft, Home, SearchX } from 'lucide-react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '@/auth/AuthContext';

export function NotFoundPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const { session } = useAuth();
  const homePath =
    session?.menus
      .filter((group) => group.enabled)
      .sort((left, right) => left.sort_order - right.sort_order)
      .flatMap((group) =>
        group.items
          .filter((item) => item.enabled)
          .sort((left, right) => left.sort_order - right.sort_order),
      )[0]?.path ?? '/login';

  return (
    <section className="flex min-h-[70vh] flex-1 items-center justify-center">
      <div className="w-full max-w-xl rounded-2xl border border-default-200 bg-content1 p-8 text-center shadow-sm sm:p-12">
        <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl bg-primary/10 text-primary">
          <SearchX className="h-8 w-8" />
        </div>
        <p className="mt-6 text-3xl font-semibold tracking-tight text-default-400">404</p>
        <h1 className="mt-3 text-xl font-semibold text-foreground">页面没有找到</h1>
        <p className="mx-auto mt-3 max-w-md text-small leading-6 text-default-500">
          当前地址尚未配置对应页面，可能是菜单路由填写错误、页面尚未上线，或地址已经发生变化。
        </p>
        <div className="mt-5 rounded-xl bg-default-100 px-4 py-3 font-mono text-tiny text-default-500">
          {location.pathname}
        </div>
        <div className="mt-7 flex flex-wrap justify-center gap-3">
          <Button
            variant="flat"
            startContent={<ArrowLeft className="h-4 w-4" />}
            onPress={() => navigate(-1)}
          >
            返回上页
          </Button>
          <Button
            color="primary"
            startContent={<Home className="h-4 w-4" />}
            onPress={() => navigate(homePath, { replace: true })}
          >
            返回首页
          </Button>
        </div>
      </div>
    </section>
  );
}
