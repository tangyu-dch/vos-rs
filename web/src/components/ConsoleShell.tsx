import { useState, type ReactNode } from 'react';
import {
  Dropdown, DropdownTrigger, DropdownMenu, DropdownItem, Button, Avatar, Chip,
  Tooltip, Divider,
  Modal, ModalContent, ModalBody,
} from '@heroui/react';
import {
  LayoutDashboard, PhoneCall, Users, BookOpen, GitBranch, GitFork, Bot,
  Grid, Server, ShieldCheck, ShieldAlert, Settings, LogOut, ChevronDown, Menu as MenuIcon, Activity, Cpu,
  Sun, Moon, ChevronsLeft, ChevronsRight,
} from 'lucide-react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '@/auth/AuthContext';
import { canAccessPage, roleLabel, type UserRole } from '@/services/auth';
import { useTheme } from '@/theme/ThemeContext';

interface NavItem {
  to: string;
  label: string;
  icon: ReactNode;
}

interface NavGroup {
  label: string;
  icon: ReactNode;
  items: NavItem[];
}

const groups: NavGroup[] = [
  { label: '运行中心', icon: <Activity className="w-3.5 h-3.5" />, items: [
    { to: '/overview', label: '运行总览', icon: <LayoutDashboard className="w-4 h-4" /> },
    { to: '/copilot', label: '智能助手', icon: <Bot className="w-4 h-4" /> },
    { to: '/calls/active', label: '活跃通话', icon: <PhoneCall className="w-4 h-4" /> },
  ] },
  { label: '号码与分机', icon: <Users className="w-3.5 h-3.5" />, items: [
    { to: '/extensions', label: '分机管理', icon: <Users className="w-4 h-4" /> },
    { to: '/numbers', label: '号码管理', icon: <BookOpen className="w-4 h-4" /> },
    { to: '/did-destinations', label: '呼入目标', icon: <GitBranch className="w-4 h-4" /> },
  ] },
  { label: '呼叫中心', icon: <Grid className="w-3.5 h-3.5" />, items: [
    { to: '/ivr', label: 'IVR 导航', icon: <GitBranch className="w-4 h-4" /> },
    { to: '/queues', label: '呼叫队列', icon: <Grid className="w-4 h-4" /> },
    { to: '/agents', label: '座席监控', icon: <Users className="w-4 h-4" /> },
  ] },
  { label: '中继与路由', icon: <Server className="w-3.5 h-3.5" />, items: [
    { to: '/routing', label: '路由策略', icon: <GitFork className="w-4 h-4" /> },
    { to: '/trunks/access', label: '接入中继', icon: <Server className="w-4 h-4" /> },
    { to: '/trunks/egress', label: '落地中继', icon: <Server className="w-4 h-4" /> },
    { to: '/egress-groups', label: '落地分组', icon: <GitBranch className="w-4 h-4" /> },
    { to: '/caller-pools', label: '号码池组', icon: <Grid className="w-4 h-4" /> },
  ] },
  { label: '通话分析', icon: <PhoneCall className="w-3.5 h-3.5" />, items: [
    { to: '/calls', label: '通话记录', icon: <PhoneCall className="w-4 h-4" /> },
  ] },
  { label: '计费中心', icon: <BookOpen className="w-3.5 h-3.5" />, items: [
    { to: '/billing/accounts', label: '计费账户', icon: <Users className="w-4 h-4" /> },
    { to: '/billing/rates', label: '费率管理', icon: <Grid className="w-4 h-4" /> },
    { to: '/billing/transactions', label: '账务流水', icon: <BookOpen className="w-4 h-4" /> },
  ] },
  { label: '系统与安全', icon: <ShieldCheck className="w-3.5 h-3.5" />, items: [
    { to: '/security', label: '安全策略', icon: <ShieldCheck className="w-4 h-4" /> },
    { to: '/infrastructure', label: '集群节点', icon: <ShieldAlert className="w-4 h-4" /> },
    { to: '/settings/llm', label: 'LLM 配置', icon: <Cpu className="w-4 h-4" /> },
    { to: '/settings', label: '系统设置', icon: <Settings className="w-4 h-4" /> },
  ] },
];

/** 判断 path 是否匹配当前路由（最长前缀匹配，避免 /settings/llm 同时命中 /settings） */
function useIsActive() {
  const location = useLocation();
  // 计算当前路径的最佳匹配（最长前缀），仅该路径被视为 active
  const allPaths = groups.flatMap((g) => g.items.map((i) => i.to));
  const bestMatch = allPaths
    .filter((p) => location.pathname === p || location.pathname.startsWith(`${p}/`))
    .sort((a, b) => b.length - a.length)[0];
  return (path: string) => path === bestMatch;
}

interface NavigationProps {
  role: UserRole;
  collapsed?: boolean;
  close?: () => void;
}

function Navigation({ role, collapsed = false, close }: NavigationProps) {
  const navigate = useNavigate();
  const isActive = useIsActive();
  const visibleGroups = groups.map((group) => ({
    ...group,
    items: group.items.filter((item) => canAccessPage(role, item.to)),
  })).filter((group) => group.items.length > 0);

  const handleNavigate = (to: string) => {
    navigate(to);
    if (close) close();
  };

  if (collapsed) {
    // 折叠态：仅显示图标，鼠标悬浮显示 Tooltip
    return (
      <nav className="flex flex-col gap-2 p-2 w-full">
        {visibleGroups.map((group, groupIdx) => (
          <div key={group.label} className="flex flex-col gap-1">
            {groupIdx > 0 && <Divider className="my-1" />}
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
                      ${active
                        ? 'bg-primary text-foreground'
                        : 'text-default-500 hover:text-foreground hover:bg-default-100'}`}
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
    <nav className="flex flex-col gap-2 p-3 w-full">
      {visibleGroups.map((group, groupIdx) => (
        <div key={group.label} className="flex flex-col gap-1">
          {groupIdx > 0 && <Divider className="my-1" />}
          <div className="text-tiny font-semibold text-default-400 uppercase tracking-wider px-3 py-1.5 mb-0.5 flex items-center gap-1.5">
            {group.icon}
            <span>{group.label}</span>
          </div>
          {group.items.map((item) => {
            const active = isActive(item.to);
            return (
              <button
                key={item.to}
                type="button"
                onClick={() => handleNavigate(item.to)}
                aria-current={active ? 'page' : undefined}
                className={`relative h-10 px-3 flex items-center gap-3 rounded-medium transition-all w-full text-left
                  ${active
                    ? 'bg-primary/15 text-primary font-semibold'
                    : 'text-default-600 hover:text-foreground hover:bg-default-100 font-medium'}`}
              >
                {active && (
                  <span
                    aria-hidden
                    className="absolute left-0 top-1.5 bottom-1.5 w-1 rounded-full bg-primary"
                  />
                )}
                <span className={`flex items-center ${active ? 'text-primary' : 'text-default-500'}`}>
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
  const { session, logout } = useAuth();
  const { theme, toggleTheme } = useTheme();
  const allItems = groups.flatMap((group) => group.items);
  const active = allItems
    .filter((item) => location.pathname === item.to || location.pathname.startsWith(`${item.to}/`))
    .sort((a, b) => b.to.length - a.to.length)[0];

  const sidebarWidth = collapsed ? 'w-[68px] min-w-[68px] max-w-[68px]' : 'w-60 shrink-0';

  const sidebarHeader = (hidden: boolean) => (
    <div className={`h-16 border-b border-default-100 flex items-center shrink-0 ${hidden ? 'px-2' : 'px-5'}`}>
      <div className="flex items-center gap-3 overflow-hidden">
        <div className="w-9 h-9 rounded-medium bg-primary flex items-center justify-center font-black text-background text-xl shrink-0">
          V
        </div>
        {!hidden && (
          <div className="min-w-0">
            <strong className="block text-small font-bold tracking-tight text-foreground leading-tight truncate">VOS Console</strong>
            <small className="block text-tiny font-medium text-primary tracking-wider">SOFTSWITCH v1.0</small>
          </div>
        )}
      </div>
    </div>
  );

  return (
    <div className="flex h-screen w-screen overflow-hidden font-sans text-foreground bg-content1">
      {/* 桌面侧边栏（sm 及以上） */}
      <aside className={`hidden sm:flex ${sidebarWidth} h-screen flex-col bg-content1 border-r border-default-200 transition-[width] duration-200 z-20`}>
        {sidebarHeader(collapsed)}
        <div className="flex-1 overflow-y-auto">
          {session && <Navigation role={session.role} collapsed={collapsed} />}
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
            {collapsed ? <ChevronsRight className="w-4 h-4" /> : <ChevronsLeft className="w-4 h-4" />}
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
          base: "sm:hidden max-w-[280px] h-screen m-0 rounded-none",
          wrapper: "items-start justify-start",
        }}
      >
        <ModalContent>
          <ModalBody className="p-0 overflow-y-auto h-full">
            {sidebarHeader(false)}
            {session && <Navigation role={session.role} close={() => setMobileOpen(false)} />}
          </ModalBody>
        </ModalContent>
      </Modal>

      {/* 右侧主工作区 */}
      <div className="flex-1 flex flex-col min-w-0 h-screen overflow-hidden">
        {/* 顶栏 Header */}
        <header className="h-16 flex-none bg-content1/90 backdrop-blur-md border-b border-default-200 px-4 sm:px-6 flex items-center justify-between gap-4 z-10">
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
              <strong className="text-small font-semibold text-foreground truncate">{active?.label || '概览'}</strong>
            </div>
          </div>

          <div className="flex items-center gap-2 sm:gap-3 shrink-0">
            <Chip
              color="success"
              variant="dot"
              size="sm"
              className="hidden md:flex"
            >
              集群正常
            </Chip>

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
                      <div className="text-tiny font-semibold leading-tight text-foreground">{session.username}</div>
                      <div className="text-tiny text-default-400 leading-tight">{roleLabel(session.role)}</div>
                    </div>
                    <ChevronDown className="w-3.5 h-3.5 text-default-400 hidden sm:block" />
                  </Button>
                </DropdownTrigger>
                <DropdownMenu aria-label="用户菜单" onAction={(key) => key === 'logout' && logout()}>
                  <DropdownItem key="user" className="h-14 gap-2" isReadOnly>
                    <p className="text-tiny text-default-400">已登录为</p>
                    <p className="text-tiny font-semibold text-primary">{session.username}</p>
                  </DropdownItem>
                  <DropdownItem key="logout" color="danger" startContent={<LogOut className="w-4 h-4" />}>
                    退出登录
                  </DropdownItem>
                </DropdownMenu>
              </Dropdown>
            )}
          </div>
        </header>

        <main className="flex-1 p-4 sm:p-5 md:p-6 overflow-y-auto flex flex-col min-h-0 bg-content2">
          {children}
        </main>
      </div>
    </div>
  );
}
