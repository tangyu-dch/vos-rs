import { Avatar, Card, CardBody, Chip, Divider } from '@heroui/react';
import { BadgeCheck, KeyRound, ShieldCheck, UserRound } from 'lucide-react';
import { useAuth } from '@/auth/AuthContext';
import { roleLabel } from '@/services/auth';

interface ProfileFieldProps {
  label: string;
  value: string;
}

function ProfileField({ label, value }: ProfileFieldProps) {
  return (
    <div className="flex min-h-12 items-center justify-between gap-4 py-3">
      <span className="text-small text-default-500">{label}</span>
      <span className="text-right text-small font-medium text-foreground">{value}</span>
    </div>
  );
}

export function ProfilePage() {
  const { session } = useAuth();
  if (!session) return null;

  const menuCount = session.menus.reduce((total, group) => total + group.items.length, 0);
  const permissionSummary = session.permissions.includes('*')
    ? '全部权限'
    : `${session.permissions.length} 项权限`;

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-5">
      <div>
        <h1 className="text-xl font-semibold text-foreground">个人中心</h1>
        <p className="mt-1 text-small text-default-500">
          查看当前登录账户的基本资料、角色和访问状态
        </p>
      </div>

      <Card className="border border-default-200 bg-content1 shadow-sm">
        <CardBody className="flex flex-row items-center gap-4 p-5 sm:p-6">
          <Avatar
            name={session.display_name?.[0] || session.username?.[0]?.toUpperCase() || '?'}
            className="h-16 w-16 bg-primary text-large font-semibold text-primary-foreground"
          />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-lg font-semibold text-foreground">
                {session.display_name}
              </h2>
              <Chip
                size="sm"
                color="success"
                variant="flat"
                startContent={<BadgeCheck className="h-3.5 w-3.5" />}
              >
                状态正常
              </Chip>
            </div>
            <p className="mt-1 text-small text-default-500">{session.username}</p>
            <p className="mt-2 text-tiny text-default-400">
              账户资料和访问权限由系统管理员统一维护
            </p>
          </div>
        </CardBody>
      </Card>

      <div className="grid gap-5 lg:grid-cols-2">
        <Card className="border border-default-200 bg-content1 shadow-sm">
          <CardBody className="p-5">
            <div className="mb-2 flex items-center gap-2">
              <UserRound className="h-5 w-5 text-primary" />
              <h2 className="font-semibold text-foreground">基本资料</h2>
            </div>
            <Divider />
            <ProfileField label="登录账号" value={session.username} />
            <Divider />
            <ProfileField label="显示名称" value={session.display_name} />
            <Divider />
            <ProfileField label="账户状态" value="正常" />
            <Divider />
            <ProfileField label="登录状态" value="已登录" />
          </CardBody>
        </Card>

        <Card className="border border-default-200 bg-content1 shadow-sm">
          <CardBody className="p-5">
            <div className="mb-2 flex items-center gap-2">
              <ShieldCheck className="h-5 w-5 text-primary" />
              <h2 className="font-semibold text-foreground">角色权限</h2>
            </div>
            <Divider />
            <ProfileField label="所属角色" value={roleLabel(session.role, session.role_name)} />
            <Divider />
            <ProfileField label="有效权限" value={permissionSummary} />
            <Divider />
            <ProfileField label="可用菜单" value={`${menuCount} 项菜单`} />
            <Divider />
            <div className="flex min-h-12 items-center gap-3 py-3 text-small text-default-500">
              <KeyRound className="h-4 w-4 shrink-0 text-default-400" />
              <span>角色权限发生变化后，会话将自动同步最新配置。</span>
            </div>
          </CardBody>
        </Card>
      </div>
    </div>
  );
}
