import { useEffect, useMemo, useState } from 'react';
import {
  Button,
  Checkbox,
  Chip,
  Input,
  Modal,
  ModalBody,
  ModalContent,
  ModalFooter,
  ModalHeader,
  Select,
  SelectItem,
  Spinner,
  Switch,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableColumn,
  TableHeader,
  TableRow,
  Tabs,
  Tooltip,
} from '@heroui/react';
import {
  ChevronDown,
  ChevronRight,
  ListChecks,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  Trash2,
  UserRoundCog,
} from 'lucide-react';
import { toast } from 'sonner';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission, type MenuItem } from '@/services/auth';
import {
  assignUserRoles,
  createAccessRole,
  deleteAccessRole,
  getRoleOverview,
  replaceRolePermissions,
  updateAccessRole,
  type AccessOverview,
  type AccessPermission,
  type AccessRole,
} from '@/services/access-control';
import { EmptyState } from '@/components/detail-shell';

type RoleFormMode = 'create' | 'edit';

export function RolePermissionsPage() {
  const { session } = useAuth();
  const [data, setData] = useState<AccessOverview>();
  const [loading, setLoading] = useState(true);
  const [roleKey, setRoleKey] = useState('');
  const [draft, setDraft] = useState<string[]>([]);
  const [userRoles, setUserRoles] = useState<Record<string, string>>({});
  const [keyword, setKeyword] = useState('');
  const [formMode, setFormMode] = useState<RoleFormMode>();
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());

  const may = (permission: string) =>
    Boolean(
      session &&
      (hasPermission(session, permission) || session.permissions.includes('access.roles')),
    );
  const canCreate = may('access.roles.create');
  const canUpdate = may('access.roles.update');
  const canDelete = may('access.roles.delete');
  const canConfigure = may('access.roles.permissions');
  const canAssign = may('access.roles.assign');

  const refresh = async () => {
    setLoading(true);
    try {
      const overview = await getRoleOverview();
      setData(overview);
      setRoleKey((current) =>
        overview.roles.some((role) => role.role_key === current)
          ? current
          : (overview.roles[0]?.role_key ?? ''),
      );
      setUserRoles(
        Object.fromEntries(overview.users.map((user) => [user.username, user.role_key])),
      );
      setExpandedGroups(new Set(overview.menus.map((group) => group.group_key)));
    } catch {
      toast.error('角色权限加载失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, []);
  useEffect(() => {
    setDraft(data?.roles.find((role) => role.role_key === roleKey)?.permission_keys ?? []);
  }, [data, roleKey]);

  const visibleRoles = useMemo(
    () =>
      data?.roles.filter((role) =>
        `${role.name}${role.role_key}`.toLowerCase().includes(keyword.trim().toLowerCase()),
      ) ?? [],
    [data, keyword],
  );
  if (loading && !data)
    return (
      <div className="flex flex-1 items-center justify-center">
        <Spinner label="正在加载角色权限" />
      </div>
    );
  if (!data) return null;

  const selectedRole = data.roles.find((role) => role.role_key === roleKey);
  const roleUsers = data.users.filter((user) => user.role_key === roleKey);
  const wildcard = draft.includes('*');
  const changedAssignments = data.users.flatMap((user) => {
    const next = userRoles[user.username];
    return next && next !== user.role_key ? [{ username: user.username, role_key: next }] : [];
  });
  const isSelected = (permission: string) => wildcard || draft.includes(permission);
  const setPermissions = (permissions: string[], checked: boolean) =>
    setDraft((current) => {
      if (checked) return [...new Set([...current, ...permissions])];
      const removed = new Set(permissions);
      return current.filter((permission) => !removed.has(permission));
    });
  const savePermissions = async () => {
    try {
      await replaceRolePermissions(roleKey, [...new Set(['session.read', ...draft])]);
      toast.success('角色权限已保存，相关账户需重新登录');
      await refresh();
    } catch {
      toast.error('角色权限保存失败');
    }
  };
  const saveAssignments = async () => {
    try {
      await assignUserRoles(changedAssignments);
      toast.success('人员分配已保存，变更账户需重新登录');
      await refresh();
    } catch {
      toast.error('人员分配保存失败');
    }
  };
  const removeRole = async () => {
    if (!selectedRole) return;
    try {
      await deleteAccessRole(selectedRole.role_key);
      toast.success('角色已删除');
      setConfirmingDelete(false);
      await refresh();
    } catch {
      toast.error('角色删除失败，请确认角色未分配账户');
    }
  };
  const deleteDisabled =
    !canDelete || !selectedRole || selectedRole.role_key === 'admin' || roleUsers.length > 0;

  return (
    <section className="flex flex-col gap-5">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold">角色权限</h1>
          <p className="mt-1 text-small text-default-500">创建角色并配置菜单、按钮权限与所属人员</p>
        </div>
        <Button isIconOnly variant="flat" onPress={() => void refresh()} aria-label="刷新角色权限">
          <RefreshCw className="h-4 w-4" />
        </Button>
      </header>
      <div className="overview-card grid min-h-[680px] overflow-hidden lg:grid-cols-[280px_1fr]">
        <aside className="border-b border-default-200 p-4 lg:border-b-0 lg:border-r">
          <div className="flex gap-2">
            <Input
              size="sm"
              placeholder="搜索名称或编码"
              value={keyword}
              onValueChange={setKeyword}
              startContent={<Search className="h-4 w-4 text-default-400" />}
            />
            <Tooltip content={canCreate ? '创建角色' : '缺少创建角色权限'}>
              <span>
                <Button
                  isIconOnly
                  size="sm"
                  color="primary"
                  isDisabled={!canCreate}
                  onPress={() => setFormMode('create')}
                  aria-label="创建角色"
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </span>
            </Tooltip>
          </div>
          <div className="mt-4 grid gap-1">
            {visibleRoles.map((role) => (
              <button
                key={role.role_key}
                type="button"
                onClick={() => setRoleKey(role.role_key)}
                className={`rounded-xl px-3 py-3 text-left transition-colors ${roleKey === role.role_key ? 'bg-primary/10 text-primary' : 'hover:bg-default-100'}`}
              >
                <span className="flex items-center justify-between gap-2">
                  <strong className="text-small">{role.name}</strong>
                  {role.is_system && (
                    <Chip size="sm" variant="flat">
                      内置
                    </Chip>
                  )}
                </span>
                <small className="mt-1 block text-default-400">{role.role_key}</small>
              </button>
            ))}
          </div>
        </aside>
        <div className="min-w-0">
          <Tabs
            color="primary"
            variant="underlined"
            classNames={{ base: 'w-full border-b border-default-200 px-4', panel: 'p-0' }}
            aria-label="角色配置"
          >
            <Tab
              key="permissions"
              title={
                <span className="flex items-center gap-2">
                  <ListChecks className="h-4 w-4" />
                  功能权限
                </span>
              }
            >
              <div className="flex flex-wrap items-center justify-between gap-3 border-b border-default-200 px-5 py-3">
                <div>
                  <strong>{selectedRole?.name}</strong>
                  <span className="ml-2 text-tiny text-default-400">
                    {selectedRole?.description}
                  </span>
                </div>
                <div className="flex gap-2">
                  <Tooltip content={canUpdate ? '修改角色' : '缺少修改角色权限'}>
                    <span>
                      <Button
                        isIconOnly
                        size="sm"
                        variant="flat"
                        isDisabled={!canUpdate || !selectedRole}
                        onPress={() => setFormMode('edit')}
                        aria-label="修改角色"
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                    </span>
                  </Tooltip>
                  <Tooltip
                    content={
                      deleteDisabled ? '系统管理员、已分配账户或无权限时不可删除' : '删除角色'
                    }
                  >
                    <span>
                      <Button
                        isIconOnly
                        size="sm"
                        color="danger"
                        variant="flat"
                        isDisabled={deleteDisabled}
                        onPress={() => setConfirmingDelete(true)}
                        aria-label="删除角色"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </span>
                  </Tooltip>
                  <Button
                    size="sm"
                    variant="flat"
                    onPress={() =>
                      setExpandedGroups(new Set(data.menus.map((group) => group.group_key)))
                    }
                  >
                    全部展开
                  </Button>
                  <Button size="sm" variant="flat" onPress={() => setExpandedGroups(new Set())}>
                    全部收起
                  </Button>
                  <Button
                    size="sm"
                    color="primary"
                    startContent={<Save className="h-4 w-4" />}
                    isDisabled={wildcard || !canConfigure || !selectedRole}
                    onPress={() => void savePermissions()}
                  >
                    保存权限
                  </Button>
                </div>
              </div>
              {!canConfigure && (
                <div className="mx-5 mt-5 rounded-xl bg-warning-50 p-4 text-small text-warning-700">
                  当前角色仅可查看，缺少权限配置权限，所有勾选项已禁用。
                </div>
              )}
              {wildcard ? (
                <div className="m-5 rounded-xl bg-primary/10 p-4 text-small text-primary">
                  系统管理员拥有全部菜单和按钮权限，无需单独配置。
                </div>
              ) : (
                <div className="p-4">
                  <div className="grid grid-cols-[220px_1fr] border border-default-200 bg-default-100 px-4 py-3 text-small font-semibold">
                    <span>菜单页面</span>
                    <span>功能权限</span>
                  </div>
                  {data.menus.map((group) => (
                    <PermissionGroup
                      key={group.group_key}
                      group={group}
                      permissions={data.permissions}
                      expanded={expandedGroups.has(group.group_key)}
                      disabled={!canConfigure}
                      isSelected={isSelected}
                      onToggleExpanded={() =>
                        setExpandedGroups((current) => {
                          const next = new Set(current);
                          if (next.has(group.group_key)) next.delete(group.group_key);
                          else next.add(group.group_key);
                          return next;
                        })
                      }
                      onChange={setPermissions}
                    />
                  ))}
                </div>
              )}
            </Tab>
            <Tab
              key="users"
              title={
                <span className="flex items-center gap-2">
                  <UserRoundCog className="h-4 w-4" />
                  人员分配
                </span>
              }
            >
              <div className="flex items-center justify-between border-b border-default-200 px-5 py-3">
                <div>
                  <strong>账户角色</strong>
                  <p className="text-tiny text-default-500">
                    当前角色已有 {roleUsers.length} 个账户，可直接调整全部账户所属角色
                  </p>
                </div>
                <Button
                  size="sm"
                  color="primary"
                  startContent={<Save className="h-4 w-4" />}
                  isDisabled={!canAssign || changedAssignments.length === 0}
                  onPress={() => void saveAssignments()}
                >
                  保存分配
                </Button>
              </div>
              {!canAssign && (
                <div className="mx-5 mt-5 rounded-xl bg-warning-50 p-4 text-small text-warning-700">
                  缺少人员分配权限，角色选择已禁用。
                </div>
              )}
              <div className="p-5">
                <Table aria-label="角色人员分配表" removeWrapper>
                  <TableHeader>
                    <TableColumn>登录账号</TableColumn>
                    <TableColumn>显示名称</TableColumn>
                    <TableColumn>账户状态</TableColumn>
                    <TableColumn>所属角色</TableColumn>
                  </TableHeader>
                  <TableBody
                    emptyContent={
                      <EmptyState
                        icon={UserRoundCog}
                        title="暂无账户"
                        description="创建账户后可在此分配角色"
                      />
                    }
                  >
                    {data.users.map((user) => (
                      <TableRow key={user.username}>
                        <TableCell>{user.username}</TableCell>
                        <TableCell>{user.display_name}</TableCell>
                        <TableCell>
                          <Chip
                            size="sm"
                            color={user.enabled ? 'success' : 'default'}
                            variant="flat"
                          >
                            {user.enabled ? '正常' : '停用'}
                          </Chip>
                        </TableCell>
                        <TableCell>
                          <Select
                            size="sm"
                            aria-label={`${user.display_name}所属角色`}
                            selectedKeys={[userRoles[user.username] ?? user.role_key]}
                            isDisabled={!canAssign || user.username === session?.username}
                            onSelectionChange={(keys) => {
                              const next = Array.from(keys)[0];
                              if (typeof next === 'string')
                                setUserRoles((current) => ({ ...current, [user.username]: next }));
                            }}
                          >
                            {data.roles
                              .filter((role) => role.enabled)
                              .map((role) => (
                                <SelectItem key={role.role_key}>{role.name}</SelectItem>
                              ))}
                          </Select>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            </Tab>
          </Tabs>
        </div>
      </div>
      <RoleModal
        mode={formMode}
        role={formMode === 'edit' ? selectedRole : undefined}
        onClose={() => setFormMode(undefined)}
        onSaved={refresh}
      />
      <Modal isOpen={confirmingDelete} onOpenChange={(open) => !open && setConfirmingDelete(false)}>
        <ModalContent>
          <ModalHeader>删除角色</ModalHeader>
          <ModalBody>
            <p>确定删除“{selectedRole?.name}”吗？删除后无法恢复。</p>
          </ModalBody>
          <ModalFooter>
            <Button variant="light" onPress={() => setConfirmingDelete(false)}>
              取消
            </Button>
            <Button color="danger" onPress={() => void removeRole()}>
              删除
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </section>
  );
}

function PermissionGroup({
  group,
  permissions,
  expanded,
  disabled,
  isSelected,
  onToggleExpanded,
  onChange,
}: {
  group: AccessOverview['menus'][number];
  permissions: AccessPermission[];
  expanded: boolean;
  disabled: boolean;
  isSelected: (permission: string) => boolean;
  onToggleExpanded: () => void;
  onChange: (permissions: string[], checked: boolean) => void;
}) {
  const groupPermissions = [
    ...new Set(
      group.items.flatMap((item) =>
        permissionsForMenu(item, permissions).map((permission) => permission.permission_key),
      ),
    ),
  ];
  const checked = groupPermissions.length > 0 && groupPermissions.every(isSelected);
  return (
    <div className="border-x border-b border-default-200">
      <div className="grid min-h-11 grid-cols-[220px_1fr] items-center bg-default-50 px-4">
        <span className="flex items-center gap-2">
          <button
            type="button"
            onClick={onToggleExpanded}
            aria-label={expanded ? `收起${group.label}` : `展开${group.label}`}
          >
            {expanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
          </button>
          <Checkbox
            size="sm"
            isDisabled={disabled}
            isSelected={checked}
            onValueChange={(value) => onChange(groupPermissions, value)}
          >
            <strong className="text-small">{group.label}</strong>
          </Checkbox>
        </span>
      </div>
      {expanded &&
        group.items
          .filter((item) => item.enabled)
          .map((item) => (
            <PermissionRow
              key={item.item_key}
              item={item}
              permissions={permissionsForMenu(item, permissions)}
              disabled={disabled}
              isSelected={isSelected}
              onChange={onChange}
            />
          ))}
    </div>
  );
}

function PermissionRow({
  item,
  permissions,
  disabled,
  isSelected,
  onChange,
}: {
  item: MenuItem;
  permissions: AccessPermission[];
  disabled: boolean;
  isSelected: (permission: string) => boolean;
  onChange: (permissions: string[], checked: boolean) => void;
}) {
  const keys = permissions.map((permission) => permission.permission_key);
  return (
    <div className="grid min-h-12 grid-cols-[220px_1fr] items-center border-t border-default-100 px-4">
      <Checkbox
        size="sm"
        isDisabled={disabled}
        isSelected={keys.length > 0 && keys.every(isSelected)}
        onValueChange={(value) => onChange(keys, value)}
      >
        <span className="text-small">{item.label}</span>
      </Checkbox>
      <div className="flex flex-wrap gap-x-4 gap-y-2 py-2">
        {permissions.map((permission) => (
          <Checkbox
            key={permission.permission_key}
            size="sm"
            isDisabled={disabled}
            isSelected={isSelected(permission.permission_key)}
            onValueChange={(value) => onChange([permission.permission_key], value)}
          >
            <span className="text-tiny text-default-600">{permission.name}</span>
          </Checkbox>
        ))}
      </div>
    </div>
  );
}

const PERMISSION_KEYS_BY_MENU: Record<string, string[]> = {
  overview: ['overview.view'],
  rwi: [
    'calls.view',
    'calls.monitor',
    'calls.barge',
    'calls.play',
    'calls.transfer',
    'calls.terminate',
  ],
  copilot: ['copilot.use', 'copilot.execute'],
  active_calls: ['calls.view', 'calls.export', 'calls.terminate'],
  notifications: ['notifications.view', 'notifications.read', 'notifications.scan'],
  extensions: [
    'extensions.view',
    'extensions.create',
    'extensions.update',
    'extensions.delete',
    'extensions.import',
    'extensions.export',
    'registrations.view',
  ],
  numbers: [
    'numbers.view',
    'numbers.create',
    'numbers.update',
    'numbers.delete',
    'numbers.import',
    'numbers.export',
  ],
  did: ['termination.view', 'termination.manage'],
  routes: [
    'routing.view',
    'routing.create',
    'routing.update',
    'routing.delete',
    'routing.import',
    'routing.export',
    'routing.simulate',
  ],
  access_trunks: [
    'trunks.view',
    'trunks.create',
    'trunks.update',
    'trunks.delete',
    'trunks.import',
    'trunks.export',
  ],
  egress_trunks: [
    'trunks.view',
    'trunks.create',
    'trunks.update',
    'trunks.delete',
    'trunks.import',
    'trunks.export',
    'termination.view',
    'termination.manage',
  ],
  egress_groups: ['termination.view', 'termination.manage'],
  caller_pools: ['termination.view', 'termination.manage'],
  ivr: ['ivr.view', 'ivr.create', 'ivr.update', 'ivr.delete', 'ivr.prompts'],
  queues: ['queues.view', 'queues.create', 'queues.update', 'queues.delete', 'queues.export'],
  agents: ['agents.view', 'agents.create', 'agents.update', 'agents.delete', 'agents.export'],
  calls: ['calls.view', 'calls.export'],
  accounts: ['billing.accounts.view', 'billing.accounts.export', 'billing.accounts.credit'],
  rates: [
    'billing.rates.view',
    'billing.rates.create',
    'billing.rates.update',
    'billing.rates.delete',
    'billing.rates.import',
    'billing.rates.export',
  ],
  transactions: ['billing.ledger.view', 'billing.ledger.export'],
  security: ['security.view', 'security.manage', 'security.audit'],
  infrastructure: ['infrastructure.view', 'infrastructure.manage'],
  tenants: ['tenants.view', 'tenants.create', 'tenants.update', 'tenants.delete', 'tenants.export'],
  llm: ['llm.view', 'llm.create', 'llm.update', 'llm.delete', 'llm.activate'],
  access_control: [
    'access.accounts.view',
    'access.accounts.create',
    'access.accounts.update',
    'access.accounts.delete',
  ],
  role_permissions: [
    'access.roles.view',
    'access.roles.create',
    'access.roles.update',
    'access.roles.delete',
    'access.roles.permissions',
    'access.roles.assign',
  ],
  settings: ['settings.view', 'settings.manage'],
};

export function permissionKeysForMenu(itemKey: string, fallbackPermission: string): string[] {
  return PERMISSION_KEYS_BY_MENU[itemKey] ?? [fallbackPermission];
}

function permissionsForMenu(item: MenuItem, permissions: AccessPermission[]): AccessPermission[] {
  const accepted = new Set(permissionKeysForMenu(item.item_key, item.permission_key));
  return permissions.filter((permission) => accepted.has(permission.permission_key));
}

function RoleModal({
  mode,
  role,
  onClose,
  onSaved,
}: {
  mode?: RoleFormMode;
  role?: AccessRole;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [key, setKey] = useState('');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [enabled, setEnabled] = useState(true);
  useEffect(() => {
    setKey(role?.role_key ?? '');
    setName(role?.name ?? '');
    setDescription(role?.description ?? '');
    setEnabled(role?.enabled ?? true);
  }, [mode, role]);
  const save = async () => {
    try {
      if (mode === 'edit' && role)
        await updateAccessRole(role.role_key, { name, description, enabled });
      else await createAccessRole({ role_key: key, name, description });
      toast.success(mode === 'edit' ? '角色已修改' : '角色已创建，请继续分配权限');
      onClose();
      await onSaved();
    } catch {
      toast.error(mode === 'edit' ? '角色修改失败' : '角色创建失败');
    }
  };
  return (
    <Modal isOpen={Boolean(mode)} onOpenChange={(open) => !open && onClose()}>
      <ModalContent>
        <ModalHeader>{mode === 'edit' ? '修改角色' : '创建角色'}</ModalHeader>
        <ModalBody>
          <Input
            isDisabled={mode === 'edit'}
            label="角色标识"
            description="支持字母、数字、点、短横线和下划线"
            value={key}
            onValueChange={setKey}
          />
          <Input label="角色名称" value={name} onValueChange={setName} />
          <Input label="角色说明" value={description} onValueChange={setDescription} />
          {mode === 'edit' && (
            <Switch
              isSelected={enabled}
              isDisabled={role?.role_key === 'admin'}
              onValueChange={setEnabled}
            >
              启用角色
            </Switch>
          )}
        </ModalBody>
        <ModalFooter>
          <Button variant="light" onPress={onClose}>
            取消
          </Button>
          <Button
            color="primary"
            isDisabled={!key.trim() || !name.trim()}
            onPress={() => void save()}
          >
            {mode === 'edit' ? '保存' : '创建'}
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
