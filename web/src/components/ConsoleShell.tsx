import { useEffect, useState, type ReactNode } from 'react';
import {
  Dropdown,
  DropdownTrigger,
  DropdownMenu,
  DropdownItem,
  Button,
  Avatar,
  Chip,
  Tooltip,
  Modal,
  ModalContent,
  ModalBody,
} from '@heroui/react';
import {
  LayoutDashboard,
  PhoneCall,
  Users,
  BookOpen,
  GitBranch,
  GitFork,
  Bot,
  Radio,
  Grid,
  Server,
  ShieldCheck,
  ShieldAlert,
  Settings,
  LogOut,
  ChevronDown,
  Menu as MenuIcon,
  Activity,
  Cpu,
  Sun,
  Moon,
  ChevronsLeft,
  ChevronsRight,
  ChevronRight,
  User as UserIcon,
  Building2,
  Bell,
  KeyRound,
} from 'lucide-react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission, roleLabel, type AuthSession } from '@/services/auth';
import { useTheme } from '@/theme/ThemeContext';
import { api } from '@/services/client';
import { NotificationCenter } from '@/components/notification-center';

interface NavItem {
  to: string;
  label: string;
  icon: ReactNode;
}

interface NavGroup {
  key: string;
  label: string;
  icon: ReactNode;
  items: NavItem[];
}

function iconFor(key: string, small = false): ReactNode {
  const className = small ? 'w-3.5 h-3.5' : 'w-4 h-4';
  const icons: Record<string, ReactNode> = {
    activity: <Activity className={className} />,
    dashboard: <LayoutDashboard className={className} />,
    radio: <Radio className={className} />,
    bot: <Bot className={className} />,
    phone: <PhoneCall className={className} />,
    users: <Users className={className} />,
    book: <BookOpen className={className} />,
    branch: <GitBranch className={className} />,
    fork: <GitFork className={className} />,
    grid: <Grid className={className} />,
    server: <Server className={className} />,
    shield: <ShieldCheck className={className} />,
    alert: <ShieldAlert className={className} />,
    settings: <Settings className={className} />,
    cpu: <Cpu className={className} />,
    building: <Building2 className={className} />,
    bell: <Bell className={className} />,
    key: <KeyRound className={className} />,
  };
  return icons[key] ?? <Grid className={className} />;
}

function navigationGroups(session: AuthSession): NavGroup[] {
  return session.menus
    .filter((group) => group.enabled)
    .sort((left, right) => left.sort_order - right.sort_order)
    .map((group) => ({
      key: group.group_key,
      label: group.label,
      icon: iconFor(group.icon_key),
      items: group.items
        .filter((item) => item.enabled)
        .sort((left, right) => left.sort_order - right.sort_order)
        .map((item) => ({ to: item.path, label: item.label, icon: iconFor(item.icon_key) })),
    }))
    .filter((group) => group.items.length > 0);
}

/** 判断 path 是否匹配当前路由（最长前缀匹配，避免 /settings/llm 同时命中 /settings） */
function useIsActive(groups: NavGroup[]) {
  const location = useLocation();
  // 计算当前路径的最佳匹配（最长前缀），仅该路径被视为 active
  const allPaths = groups.flatMap((g) => g.items.map((i) => i.to));
  const bestMatch = allPaths
    .filter((p) => location.pathname === p || location.pathname.startsWith(`${p}/`))
    .sort((a, b) => b.length - a.length)[0];
  return (path: string) => path === bestMatch;
}

interface NavigationProps {
  session: AuthSession;
  collapsed?: boolean;
  close?: () => void;
}

function Navigation({ session, collapsed = false, close }: NavigationProps) {
  const navigate = useNavigate();
  const location = useLocation();
  const visibleGroups = navigationGroups(session);
  const isActive = useIsActive(visibleGroups);
  const currentGroupKey =
    visibleGroups
      .flatMap((group) =>
        group.items
          .filter(
            (item) => location.pathname === item.to || location.pathname.startsWith(`${item.to}/`),
          )
          .map((item) => ({ key: group.key, length: item.to.length })),
      )
      .sort((left, right) => right.length - left.length)[0]?.key ?? visibleGroups[0]?.key;
  const [extraExpanded, setExtraExpanded] = useState<string | null>(null);
  const [currentExpanded, setCurrentExpanded] = useState(true);

  useEffect(() => {
    setCurrentExpanded(true);
    setExtraExpanded(null);
  }, [currentGroupKey]);

  const handleNavigate = (to: string) => {
    navigate(to);
    if (close) close();
  };

  if (collapsed) {
    // 折叠态：仅显示图标，鼠标悬浮显示 Tooltip
    return (
      <nav className="flex flex-col gap-2 p-2 w-full">
        {visibleGroups.map((group) => (
          <div key={group.key} className="flex flex-col gap-1">
            {group.items.map((item) => {
              const active = isActive(item.to);
              return (
                <Tooltip key={item.to} content={item.label} placement="right" delay={200}>
                  <button
                    type="button"
                    onClick={() => handleNavigate(item.to)}
                    aria-label={item.label}
                    aria-current={active ? 'page' : undefined}
                    className={`w-10 h-10 mx-auto flex items-center justify-center rounded-medium transition-colors
                      ${
                        active
                          ? 'bg-primary text-foreground'
                          : 'text-default-500 hover:text-foreground hover:bg-default-100'
                      }`}
                  >
                    {item.icon}
                  </button>
                </Tooltip>
              );
            })}
          </div>
        ))}
      </nav>
    );
  }

  // 展开态：图标 + 文字 + 明确选中样式
  return (
    <nav className="flex w-full flex-col gap-1 p-3">
      {visibleGroups.map((group) => (
        <div key={group.key} className="flex flex-col gap-1">
          <button
            type="button"
            aria-expanded={
              group.key === currentGroupKey ? currentExpanded : group.key === extraExpanded
            }
            onClick={() =>
              group.key === currentGroupKey
                ? setCurrentExpanded((current) => !current)
                : setExtraExpanded((current) => (current === group.key ? null : group.key))
            }
            className={`flex h-11 w-full items-center gap-3 rounded-lg px-3 text-small font-medium transition-colors ${group.key === currentGroupKey ? 'text-primary' : 'text-default-600 hover:bg-default-100 hover:text-foreground'}`}
          >
            {group.icon}
            <span className="flex-1 text-left">{group.label}</span>
            <ChevronRight
              className={`h-4 w-4 transition-transform ${(group.key === currentGroupKey ? currentExpanded : group.key === extraExpanded) ? '-rotate-90' : 'rotate-90'}`}
            />
          </button>
          {(group.key === currentGroupKey ? currentExpanded : group.key === extraExpanded) &&
            group.items.map((item) => {
              const active = isActive(item.to);
              return (
                <button
                  key={item.to}
                  type="button"
                  onClick={() => handleNavigate(item.to)}
                  aria-current={active ? 'page' : undefined}
                  className={`relative ml-7 h-10 w-[calc(100%_-_1.75rem)] px-4 flex items-center gap-3 rounded-lg transition-all text-left
                  ${
                    active
                      ? 'bg-default-100 text-primary font-medium'
                      : 'text-default-600 hover:text-foreground hover:bg-default-100 font-medium'
                  }`}
                >
                  <span
                    className={`flex items-center ${active ? 'text-primary' : 'text-default-500'}`}
                  >
                    {item.icon}
                  </span>
                  <span className="text-small truncate">{item.label}</span>
                </button>
              );
            })}
        </div>
      ))}
    </nav>
  );
}

export default function ConsoleShell({ children }: { children: ReactNode }) {
  const [mobileOpen, setMobileOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const location = useLocation();
  const navigate = useNavigate();
  const { session, logout } = useAuth();
  const { theme, toggleTheme } = useTheme();
  const currentGroups = session ? navigationGroups(session) : [];
  const allItems = currentGroups.flatMap((group) => group.items);
  const active = allItems
    .filter((item) => location.pathname === item.to || location.pathname.startsWith(`${item.to}/`))
    .sort((a, b) => b.to.length - a.to.length)[0];
  const pageTitle = location.pathname === '/profile' ? '个人中心' : active?.label || '概览';

  // ============ 全局顶栏集群指标状态（SIP 节点数 / 媒体节点数 / 并发 / CPS） ============
  const [clusterStats, setClusterStats] = useState({
    sipNodes: 0,
    mediaNodes: 0,
    activeCalls: 0,
    cps: 0,
  });

  useEffect(() => {
    const fetchClusterStats = async () => {
      try {
        const [sipCluster, mediaCluster, summary] = await Promise.all([
          api.get<{ nodes: unknown[] }>('/infrastructure/sip-cluster').catch(() => ({ nodes: [] })),
          api
            .get<{ nodes: unknown[] }>('/infrastructure/media-cluster')
            .catch(() => ({ nodes: [] })),
          api
            .get<{ active_calls: number; today_total_calls: number }>('/overview/summary')
            .catch(() => ({ active_calls: 0, today_total_calls: 0 })),
        ]);
        setClusterStats({
          sipNodes: sipCluster.nodes?.length || 0,
          mediaNodes: mediaCluster.nodes?.length || 0,
          activeCalls: summary.active_calls || 0,
          cps: 0,
        });
      } catch {
        // 忽略
      }
    };
    void fetchClusterStats();
    const timer = setInterval(() => {
      void fetchClusterStats();
    }, 15000);
    return () => clearInterval(timer);
  }, []);

  const sidebarWidth = collapsed ? 'w-[68px] min-w-[68px] max-w-[68px]' : 'w-60 shrink-0';

  const sidebarHeader = (hidden: boolean) => (
    <div
      className={`h-16 border-b border-default-100 flex items-center shrink-0 ${hidden ? 'px-2' : 'px-5'}`}
    >
      <div className="flex items-center gap-3 overflow-hidden">
        <div className="w-9 h-9 rounded-medium bg-primary flex items-center justify-center font-black text-background text-xl shrink-0">
          V
        </div>
        {!hidden && (
          <div className="min-w-0">
            <strong className="block text-small font-bold tracking-tight text-foreground leading-tight truncate">
              话务平台
            </strong>
            <small className="block text-tiny text-default-400 tracking-wide">软交换平台</small>
          </div>
        )}
      </div>
    </div>
  );

  return (
    <div className="flex h-screen w-screen overflow-hidden font-sans text-foreground bg-content1">
      {/* 桌面侧边栏（sm 及以上） */}
      <aside
        className={`hidden sm:flex ${sidebarWidth} h-screen flex-col bg-content1 border-r border-default-200 transition-[width] duration-200 z-20`}
      >
        {sidebarHeader(collapsed)}
        <div className="flex-1 overflow-y-auto">
          {session && <Navigation session={session} collapsed={collapsed} />}
        </div>
        {/* 折叠/展开按钮（去掉 isIconOnly 避免与 w-full 冲突） */}
        <div className="border-t border-default-100 p-2 shrink-0">
          <Button
            variant="light"
            size="sm"
            className="w-full"
            onPress={() => setCollapsed((c) => !c)}
            aria-label={collapsed ? '展开侧边栏' : '收起侧边栏'}
          >
            {collapsed ? (
              <ChevronsRight className="w-4 h-4" />
            ) : (
              <ChevronsLeft className="w-4 h-4" />
            )}
          </Button>
        </div>
      </aside>

      {/* 移动端导航抽屉（sm 以下）：使用 size="sm" 避免与 max-w-[280px] 冲突 */}
      <Modal
        isOpen={mobileOpen}
        onOpenChange={setMobileOpen}
        size="sm"
        hideCloseButton
        classNames={{
          base: 'sm:hidden max-w-[280px] h-screen m-0 rounded-none',
          wrapper: 'items-start justify-start',
        }}
      >
        <ModalContent>
          <ModalBody className="p-0 overflow-y-auto h-full">
            {sidebarHeader(false)}
            {session && <Navigation session={session} close={() => setMobileOpen(false)} />}
          </ModalBody>
        </ModalContent>
      </Modal>

      {/* 右侧主工作区 */}
      <div className="flex-1 flex flex-col min-w-0 h-screen overflow-hidden">
        {/* 顶栏 Header */}
        <header className="h-14 flex-none bg-content1 border-b border-default-200 px-4 sm:px-5 flex items-center justify-between gap-4 z-10">
          <div className="flex items-center gap-3 min-w-0">
            <Button
              isIconOnly
              variant="light"
              size="sm"
              className="sm:hidden"
              onPress={() => setMobileOpen(true)}
              aria-label="打开导航菜单"
            >
              <MenuIcon className="w-5 h-5" />
            </Button>
            <div className="flex items-center gap-2 min-w-0">
              <span className="text-tiny text-default-400 font-medium shrink-0">控制台</span>
              <span className="text-default-300 shrink-0">/</span>
              <strong className="text-small font-semibold text-foreground truncate">
                {pageTitle}
              </strong>
            </div>
          </div>

          <div className="flex items-center gap-2 sm:gap-3 shrink-0">
            <div className="hidden lg:flex items-center gap-1.5 text-[11px] mr-1">
              <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-default-50 border border-default-200/70 text-default-500">
                <span className="w-1.5 h-1.5 rounded-full bg-success animate-pulse" />
                信令节点{' '}
                <strong className="text-foreground font-semibold tnum">
                  {clusterStats.sipNodes}
                </strong>
              </span>
              <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-default-50 border border-default-200/70 text-default-500">
                媒体节点{' '}
                <strong className="text-foreground font-semibold tnum">
                  {clusterStats.mediaNodes}
                </strong>
              </span>
              <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-default-50 border border-default-200/70 text-default-500">
                活跃并发{' '}
                <strong className="text-foreground font-semibold tnum">
                  {clusterStats.activeCalls}
                </strong>
              </span>
              <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-default-50 border border-default-200/70 text-default-500">
                每秒呼叫{' '}
                <strong className="text-foreground font-semibold tnum">{clusterStats.cps}</strong>
              </span>
            </div>

            <Chip color="success" variant="dot" size="sm" className="hidden md:flex">
              集群正常
            </Chip>

            {session && hasPermission(session, 'notifications.view') && <NotificationCenter />}

            <Button
              isIconOnly
              variant="light"
              size="sm"
              onPress={toggleTheme}
              aria-label="切换主题"
            >
              {theme === 'dark' ? <Sun className="w-5 h-5" /> : <Moon className="w-5 h-5" />}
            </Button>

            {session && (
              <Dropdown placement="bottom-end">
                <DropdownTrigger>
                  <Button variant="light" size="sm" className="flex items-center gap-2.5 h-9 px-2">
                    <Avatar
                      name={session.username?.[0]?.toUpperCase() || '?'}
                      size="sm"
                      className="font-bold"
                    />
                    <div className="text-left hidden sm:block">
                      <div className="text-tiny font-semibold leading-tight text-foreground">
                        {session.username}
                      </div>
                      <div className="text-tiny text-default-400 leading-tight">
                        {roleLabel(session.role, session.role_name)}
                      </div>
                    </div>
                    <ChevronDown className="w-3.5 h-3.5 text-default-400 hidden sm:block" />
                  </Button>
                </DropdownTrigger>
                <DropdownMenu
                  aria-label="用户菜单"
                  variant="flat"
                  onAction={(key) => {
                    if (key === 'profile') navigate('/profile');
                    if (key === 'notifications') navigate('/notifications');
                    if (key === 'logout') logout();
                  }}
                >
                  <DropdownItem
                    key="profile"
                    startContent={<UserIcon className="h-4 w-4 text-default-500" />}
                  >
                    个人中心
                  </DropdownItem>
                  <DropdownItem
                    key="notifications"
                    showDivider
                    startContent={<Bell className="h-4 w-4 text-default-500" />}
                  >
                    消息中心
                  </DropdownItem>
                  <DropdownItem
                    key="logout"
                    startContent={<LogOut className="h-4 w-4 text-default-500" />}
                  >
                    退出登录
                  </DropdownItem>
                </DropdownMenu>
              </Dropdown>
            )}
          </div>
        </header>

        <main className="flex-1 p-4 sm:p-5 overflow-y-auto flex flex-col min-h-0 bg-default-50/60">
          {children}
        </main>
      </div>
    </div>
  );
}
