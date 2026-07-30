import { useEffect, useMemo, useState } from 'react';
import {
  Button,
  Chip,
  Input,
  Modal,
  ModalBody,
  ModalContent,
  ModalFooter,
  ModalHeader,
  Pagination,
  Select,
  SelectItem,
  Spinner,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableColumn,
  TableHeader,
  TableRow,
  Tooltip,
} from '@heroui/react';
import { Pencil, Plus, RefreshCw, Search, Trash2, UserCog, Users } from 'lucide-react';
import { toast } from 'sonner';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission } from '@/services/auth';
import {
  createConsoleUser,
  deleteConsoleUser,
  getAccountOverview,
  updateConsoleUser,
  type AccessOverview,
  type ConsoleUser,
} from '@/services/access-control';
import { EmptyState } from '@/components/detail-shell';

const PAGE_SIZE = 10;

export function AccessAccountsPage() {
  const { session } = useAuth();
  const [data, setData] = useState<AccessOverview>();
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<ConsoleUser>();
  const [deleting, setDeleting] = useState<ConsoleUser>();
  const [creating, setCreating] = useState(false);
  const [keyword, setKeyword] = useState('');
  const [roleFilter, setRoleFilter] = useState('all');
  const [statusFilter, setStatusFilter] = useState('all');
  const [page, setPage] = useState(1);
  const may = (permission: string) =>
    Boolean(
      session &&
      (hasPermission(session, permission) || session.permissions.includes('access.users')),
    );
  const canCreate = may('access.accounts.create');
  const canUpdate = may('access.accounts.update');
  const canDelete = may('access.accounts.delete');

  const refresh = async () => {
    setLoading(true);
    try {
      setData(await getAccountOverview());
    } catch {
      toast.error('账户信息加载失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, []);
  useEffect(() => {
    setPage(1);
  }, [keyword, roleFilter, statusFilter]);

  const filteredUsers = useMemo(() => {
    const normalizedKeyword = keyword.trim().toLowerCase();
    return (data?.users ?? []).filter((user) => {
      const matchesKeyword =
        !normalizedKeyword ||
        `${user.username}${user.display_name}${user.role_name}`
          .toLowerCase()
          .includes(normalizedKeyword);
      const matchesRole = roleFilter === 'all' || user.role_key === roleFilter;
      const matchesStatus =
        statusFilter === 'all' || (statusFilter === 'enabled' ? user.enabled : !user.enabled);
      return matchesKeyword && matchesRole && matchesStatus;
    });
  }, [data?.users, keyword, roleFilter, statusFilter]);

  const totalPages = Math.max(1, Math.ceil(filteredUsers.length / PAGE_SIZE));
  const visibleUsers = filteredUsers.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
  const activeCount = data?.users.filter((user) => user.enabled).length ?? 0;

  if (loading && !data) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Spinner label="正在加载账户" />
      </div>
    );
  }
  if (!data) return null;

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold text-foreground">用户管理</h1>
          <p className="mt-1 text-small text-default-500">
            管理控制台登录账户、所属角色、登录状态和认证密码
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            isIconOnly
            variant="flat"
            isLoading={loading}
            onPress={() => void refresh()}
            aria-label="刷新账户"
          >
            <RefreshCw className="h-4 w-4" />
          </Button>
          <Tooltip content={canCreate ? '新建账户' : '缺少新增账户权限'}>
            <span>
              <Button
                color="primary"
                isDisabled={!canCreate}
                startContent={<Plus className="h-4 w-4" />}
                onPress={() => setCreating(true)}
              >
                新建账户
              </Button>
            </span>
          </Tooltip>
        </div>
      </header>

      <div className="grid gap-3 sm:grid-cols-3">
        <SummaryCard label="账户总数" value={data.users.length} />
        <SummaryCard label="正常账户" value={activeCount} color="success" />
        <SummaryCard label="角色数量" value={data.roles.filter((role) => role.enabled).length} />
      </div>

      <div className="overview-card overflow-hidden">
        <div className="flex flex-wrap items-center gap-3 border-b border-default-200 p-4">
          <Input
            className="min-w-60 flex-1 sm:max-w-80"
            size="sm"
            placeholder="搜索账号、名称或角色"
            value={keyword}
            onValueChange={setKeyword}
            startContent={<Search className="h-4 w-4 text-default-400" />}
            isClearable
            onClear={() => setKeyword('')}
          />
          <Select
            className="w-44"
            size="sm"
            aria-label="所属角色"
            selectedKeys={[roleFilter]}
            onSelectionChange={(keys) => setRoleFilter(String(Array.from(keys)[0] ?? 'all'))}
          >
            {[{ role_key: 'all', name: '全部角色' }, ...data.roles].map((role) => (
              <SelectItem key={role.role_key}>{role.name}</SelectItem>
            ))}
          </Select>
          <Select
            className="w-36"
            size="sm"
            aria-label="账户状态"
            selectedKeys={[statusFilter]}
            onSelectionChange={(keys) => setStatusFilter(String(Array.from(keys)[0] ?? 'all'))}
          >
            <SelectItem key="all">全部状态</SelectItem>
            <SelectItem key="enabled">正常使用</SelectItem>
            <SelectItem key="disabled">已经停用</SelectItem>
          </Select>
        </div>

        <Table aria-label="控制台账户列表" removeWrapper>
          <TableHeader>
            <TableColumn>登录账号</TableColumn>
            <TableColumn>显示名称</TableColumn>
            <TableColumn>所属角色</TableColumn>
            <TableColumn>账户类型</TableColumn>
            <TableColumn>账户状态</TableColumn>
            <TableColumn align="end">操作</TableColumn>
          </TableHeader>
          <TableBody
            items={visibleUsers}
            emptyContent={
              <EmptyState
                icon={Users}
                title="没有匹配的账户"
                description="调整搜索内容或筛选条件后再试"
              />
            }
          >
            {(user) => (
              <TableRow key={user.username}>
                <TableCell>
                  <span className="font-mono font-medium text-foreground">{user.username}</span>
                </TableCell>
                <TableCell>{user.display_name}</TableCell>
                <TableCell>
                  <Chip size="sm" variant="flat" color="primary">
                    {user.role_name}
                  </Chip>
                </TableCell>
                <TableCell>{user.is_builtin ? '内置账户' : '普通账户'}</TableCell>
                <TableCell>
                  <Chip size="sm" color={user.enabled ? 'success' : 'default'} variant="flat">
                    {user.enabled ? '正常使用' : '已经停用'}
                  </Chip>
                </TableCell>
                <TableCell>
                  <div className="flex justify-end gap-1">
                    <Tooltip content={canUpdate ? '编辑账户' : '缺少修改账户权限'}>
                      <span>
                        <Button
                          isIconOnly
                          size="sm"
                          variant="light"
                          isDisabled={!canUpdate}
                          onPress={() => setEditing(user)}
                          aria-label={`编辑${user.username}`}
                        >
                          <Pencil className="h-4 w-4" />
                        </Button>
                      </span>
                    </Tooltip>
                    <Tooltip
                      content={
                        user.is_builtin || user.username === session?.username
                          ? '内置账户或当前账户不能删除'
                          : canDelete
                            ? '删除账户'
                            : '缺少删除账户权限'
                      }
                    >
                      <span>
                        <Button
                          isIconOnly
                          size="sm"
                          color="danger"
                          variant="light"
                          isDisabled={
                            !canDelete || user.is_builtin || user.username === session?.username
                          }
                          onPress={() => setDeleting(user)}
                          aria-label={`删除${user.username}`}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </span>
                    </Tooltip>
                  </div>
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>

        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-default-200 px-4 py-3">
          <span className="text-tiny text-default-500">共 {filteredUsers.length} 个账户</span>
          {totalPages > 1 && (
            <Pagination size="sm" page={page} total={totalPages} onChange={setPage} />
          )}
        </div>
      </div>

      <AccountModal
        user={editing}
        roles={data.roles}
        currentUsername={session?.username}
        open={creating || Boolean(editing)}
        onClose={() => {
          setCreating(false);
          setEditing(undefined);
        }}
        onSaved={refresh}
      />
      <DeleteAccountModal
        user={deleting}
        onClose={() => setDeleting(undefined)}
        onDeleted={refresh}
      />
    </section>
  );
}

function DeleteAccountModal({
  user,
  onClose,
  onDeleted,
}: {
  user?: ConsoleUser;
  onClose: () => void;
  onDeleted: () => Promise<void>;
}) {
  const [deleting, setDeleting] = useState(false);
  const remove = async () => {
    if (!user) return;
    setDeleting(true);
    try {
      await deleteConsoleUser(user.username);
      toast.success('账户已删除');
      onClose();
      await onDeleted();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : '账户删除失败');
    } finally {
      setDeleting(false);
    }
  };
  return (
    <Modal isOpen={Boolean(user)} onOpenChange={(open) => !open && onClose()}>
      <ModalContent>
        <ModalHeader>删除账户</ModalHeader>
        <ModalBody>
          <p>
            确定删除“{user?.display_name}（{user?.username}）”吗？删除后无法恢复。
          </p>
        </ModalBody>
        <ModalFooter>
          <Button variant="light" onPress={onClose}>
            取消
          </Button>
          <Button color="danger" isLoading={deleting} onPress={() => void remove()}>
            删除
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}

function SummaryCard({
  label,
  value,
  color = 'primary',
}: {
  label: string;
  value: number;
  color?: 'primary' | 'success';
}) {
  const iconColor =
    color === 'success' ? 'bg-success/10 text-success' : 'bg-primary/10 text-primary';
  return (
    <div className="overview-card flex items-center gap-3 p-4">
      <span className={`flex h-10 w-10 items-center justify-center rounded-xl ${iconColor}`}>
        <UserCog className="h-5 w-5" />
      </span>
      <span>
        <small className="block text-default-500">{label}</small>
        <strong className="text-lg font-semibold tnum">{value}</strong>
      </span>
    </div>
  );
}

function AccountModal({
  user,
  roles,
  currentUsername,
  open,
  onClose,
  onSaved,
}: {
  user?: ConsoleUser;
  roles: AccessOverview['roles'];
  currentUsername?: string;
  open: boolean;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [username, setUsername] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const [roleKey, setRoleKey] = useState('');
  const [enabled, setEnabled] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setUsername(user?.username ?? '');
    setDisplayName(user?.display_name ?? '');
    setPassword('');
    setRoleKey(user?.role_key ?? roles.find((role) => role.enabled)?.role_key ?? '');
    setEnabled(user?.enabled ?? true);
  }, [user, roles, open]);

  const usernameValid = /^[A-Za-z0-9._-]{1,64}$/.test(username);
  const passwordValid =
    Boolean(user && !password) || (password.length >= 10 && password.length <= 128);
  const formValid =
    usernameValid &&
    displayName.trim().length > 0 &&
    displayName.trim().length <= 64 &&
    Boolean(roleKey) &&
    passwordValid;

  const save = async () => {
    setSaving(true);
    try {
      if (user) {
        await updateConsoleUser(user.username, {
          display_name: displayName.trim(),
          role_key: roleKey,
          enabled,
          ...(password ? { password } : {}),
        });
      } else {
        await createConsoleUser({
          username,
          display_name: displayName.trim(),
          password,
          role_key: roleKey,
        });
      }
      toast.success('账户配置已保存');
      onClose();
      await onSaved();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : '账户配置保存失败');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal isOpen={open} onOpenChange={(value) => !value && onClose()}>
      <ModalContent>
        <ModalHeader>{user ? '编辑账户' : '新建账户'}</ModalHeader>
        <ModalBody>
          <Input
            label="登录账号"
            description="支持字母、数字、点、短横线和下划线"
            value={username}
            onValueChange={setUsername}
            isDisabled={Boolean(user)}
            isInvalid={Boolean(username) && !usernameValid}
            errorMessage="账号格式不正确"
          />
          <Input label="显示名称" value={displayName} onValueChange={setDisplayName} />
          <Input
            label={user ? '新密码（留空不变）' : '登录密码'}
            description="密码长度为 10 到 128 个字符"
            type="password"
            value={password}
            onValueChange={setPassword}
            isInvalid={Boolean(password) && !passwordValid}
            errorMessage="密码长度不符合要求"
          />
          <Select
            label="所属角色"
            selectedKeys={roleKey ? [roleKey] : []}
            onSelectionChange={(keys) => setRoleKey(String(Array.from(keys)[0] ?? ''))}
          >
            {roles
              .filter((role) => role.enabled || role.role_key === user?.role_key)
              .map((role) => (
                <SelectItem key={role.role_key}>{role.name}</SelectItem>
              ))}
          </Select>
          {user && (
            <Switch
              isSelected={enabled}
              onValueChange={setEnabled}
              isDisabled={user.username === currentUsername}
            >
              {user.username === currentUsername ? '当前账户不能停用' : '允许登录'}
            </Switch>
          )}
        </ModalBody>
        <ModalFooter>
          <Button variant="light" onPress={onClose}>
            取消
          </Button>
          <Button
            color="primary"
            isLoading={saving}
            isDisabled={!formValid}
            onPress={() => void save()}
          >
            保存
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
