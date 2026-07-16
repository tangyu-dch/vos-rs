import { useEffect, useState, type ReactNode } from 'react';
import { Button, Drawer, Dropdown, Menu, Tooltip } from '@arco-design/web-react';
import {
  IconApps, IconBook, IconBranch, IconBug, IconDashboard, IconDown, IconMenu,
  IconPhone, IconSafe, IconSettings, IconStorage, IconUser, IconUserGroup,
} from '@arco-design/web-react/icon';
import { NavLink, useLocation } from 'react-router-dom';
import { useAuth } from '../auth/AuthContext';
import { canAccessPage, roleLabel, type UserRole } from '../services/auth';

const groups = [
  { label: '运行中心', items: [
    { to: '/overview', label: '运行总览', icon: <IconDashboard /> },
    { to: '/calls/active', label: '活跃通话', icon: <IconPhone /> },
  ] },
  { label: '号码分机', items: [
    { to: '/extensions', label: '分机管理', icon: <IconUserGroup /> },
    { to: '/numbers', label: '号码管理', icon: <IconBook /> },
  ] },
  { label: '中继路由', items: [
    { to: '/trunks', label: '中继管理', icon: <IconStorage /> },
    { to: '/routing', label: '路由管理', icon: <IconBranch /> },
  ] },
  { label: '通话分析', items: [{ to: '/calls', label: '通话记录', icon: <IconPhone /> }] },
  { label: '计费中心', items: [
    { to: '/billing/accounts', label: '计费账户', icon: <IconUser /> },
    { to: '/billing/rates', label: '费率管理', icon: <IconApps /> },
    { to: '/billing/transactions', label: '账务流水', icon: <IconBook /> },
  ] },
  { label: '安全系统', items: [
    { to: '/security', label: '安全策略', icon: <IconSafe /> },
    { to: '/infrastructure', label: '集群节点', icon: <IconBug /> },
    { to: '/settings', label: '系统设置', icon: <IconSettings /> },
  ] },
];

function Navigation({ role, close }: { role: UserRole; close?: () => void }) {
  const location = useLocation();
  const visibleGroups = groups.map((group) => ({ ...group, items: group.items.filter((item) => canAccessPage(role, item.to)) })).filter((group) => group.items.length > 0);
  const activeGroup = visibleGroups.find((group) => group.items.some((item) => location.pathname.startsWith(item.to)))?.label;
  const [expanded, setExpanded] = useState(() => new Set(activeGroup ? [activeGroup] : [visibleGroups[0]?.label]));
  useEffect(() => {
    if (activeGroup) setExpanded((current) => new Set(current).add(activeGroup));
  }, [activeGroup]);
  const toggle = (label: string) => setExpanded((current) => {
    const next = new Set(current);
    if (next.has(label)) next.delete(label); else next.add(label);
    return next;
  });
  const isCurrent = (path: string) => path === '/calls'
    ? location.pathname === path || (location.pathname.startsWith('/calls/') && !location.pathname.startsWith('/calls/active'))
    : location.pathname === path || location.pathname.startsWith(`${path}/`);
  return <nav className="console-nav" aria-label="主导航">{visibleGroups.map((group) => {
    const isExpanded = expanded.has(group.label);
    return <section key={group.label} className={isExpanded ? 'expanded' : ''}>
      <button type="button" className="console-nav-group" aria-expanded={isExpanded} onClick={() => toggle(group.label)}>
        <span>{group.label}</span><IconDown />
      </button>
      <div className="console-nav-items">{group.items.map((item) => (
        <NavLink key={item.to} to={item.to} onClick={close} className={isCurrent(item.to) ? 'active' : ''}>
          {item.icon}<span>{item.label}</span>
        </NavLink>
      ))}</div>
    </section>;
  })}</nav>;
}

export default function ConsoleShell({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const location = useLocation();
  const { session, logout } = useAuth();
  const active = groups.flatMap((group) => group.items).find((item) => location.pathname.startsWith(item.to));
  const userMenu = <Menu><Menu.Item key="logout" onClick={logout}>退出登录</Menu.Item></Menu>;
  return <div className="console-shell">
    <aside className="console-sidebar">
      <div className="console-brand"><span className="brand-mark">V</span><div><strong>VOS Console</strong><small>Softswitch Control</small></div></div>
      {session && <Navigation role={session.role} />}
    </aside>
    <div className="console-body">
      <header className="console-topbar">
        <Tooltip content="打开导航"><Button className="mobile-menu" icon={<IconMenu />} onClick={() => setOpen(true)} /></Tooltip>
        <div><span className="topbar-context">工作区</span><strong>{active?.label || '控制台'}</strong></div>
        <div className="topbar-actions"><Dropdown droplist={userMenu} position="br"><Button type="text">{session?.username} · {session ? roleLabel(session.role) : ''}<IconDown /></Button></Dropdown></div>
      </header>
      <main className="console-main">{children}</main>
    </div>
    <Drawer className="mobile-drawer" width={280} visible={open} onCancel={() => setOpen(false)} footer={null} title="VOS Console">{session && <Navigation role={session.role} close={() => setOpen(false)} />}</Drawer>
  </div>;
}
