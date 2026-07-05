import { ReactNode, useState } from 'react';
import {
  Layout as ArcoLayout,
  Menu,
  Breadcrumb,
  Avatar,
  Dropdown,
  Button,
  Tag,
} from '@arco-design/web-react';
import {
  IconDashboard,
  IconPhone,
  IconFile,
  IconStorage,
  IconBranch,
  IconShareAlt,
  IconSettings,
  IconUserGroup,
  IconUser,
  IconIdcard,
  IconVideoCamera,
  IconCommon,
  IconDriveFile,
  IconMenuFold,
  IconMenuUnfold,
  IconDown,
} from '@arco-design/web-react/icon';
import { useLocation, useNavigate } from 'react-router-dom';
import './Layout.css';

const { Header, Sider, Content } = ArcoLayout;
const MenuItem = Menu.Item;

interface LayoutProps {
  children: ReactNode;
}

interface NavItem {
  key: string;
  icon: ReactNode;
  title: string;
  desc: string;
  group: string;
}

const NAV_ITEMS: NavItem[] = [
  { key: '/dashboard', icon: <IconDashboard />, title: '仪表盘', desc: '总览与监控', group: '监控运营' },
  { key: '/active-calls', icon: <IconPhone />, title: '活跃呼叫', desc: '实时通话', group: '监控运营' },
  { key: '/cdr', icon: <IconFile />, title: '呼叫记录', desc: 'CDR 明细', group: '监控运营' },
  { key: '/recordings', icon: <IconVideoCamera />, title: '录音', desc: '试听与下载', group: '监控运营' },
  { key: '/reports', icon: <IconCommon />, title: '报表', desc: '统计与导出', group: '监控运营' },
  { key: '/users', icon: <IconUserGroup />, title: 'SIP 用户', desc: '账户管理', group: '号码路由' },
  { key: '/gateways', icon: <IconStorage />, title: '网关管理', desc: '中继网关', group: '号码路由' },
  { key: '/routes', icon: <IconBranch />, title: '路由管理', desc: '选路规则', group: '号码路由' },
  { key: '/registrations', icon: <IconShareAlt />, title: '注册信息', desc: '在线终端', group: '号码路由' },
  { key: '/numbers', icon: <IconIdcard />, title: '号码库存', desc: '号码分配', group: '号码路由' },
  { key: '/rates', icon: <IconDriveFile />, title: '费率', desc: '计费费率', group: '计费' },
  { key: '/accounts', icon: <IconUser />, title: '账户', desc: '余额与对账', group: '计费' },
];

const NAV_GROUPS = ['监控运营', '号码路由', '计费'];

export default function Layout({ children }: LayoutProps) {
  const [collapsed, setCollapsed] = useState(false);
  const location = useLocation();
  const navigate = useNavigate();

  const selectedKey =
    location.pathname === '/' ? '/dashboard' : location.pathname;
  const activeNav =
    NAV_ITEMS.find((m) => m.key === selectedKey) || NAV_ITEMS[0];

  const userDropdown = (
    <Menu>
      <MenuItem key="settings">
        <IconSettings style={{ marginRight: 8 }} />
        系统设置
      </MenuItem>
    </Menu>
  );

  return (
    <ArcoLayout className="app-layout">
      <Sider
        collapsed={collapsed}
        collapsible
        trigger={null}
        breakpoint="lg"
        width={236}
        collapsedWidth={72}
        onCollapse={setCollapsed}
        className="app-sider"
      >
        <div className={`sider-brand ${collapsed ? 'is-collapsed' : ''}`}>
          <div className="sider-brand__logo">
            <svg viewBox="0 0 32 32" width="22" height="22" fill="none">
              <rect width="32" height="32" rx="8" fill="url(#g)" />
              <path
                d="M9 11.5a3.5 3.5 0 0 1 3.5-3.5h7A3.5 3.5 0 0 1 23 11.5v9A3.5 3.5 0 0 1 19.5 24h-7A3.5 3.5 0 0 1 9 20.5v-9Z"
                stroke="#fff"
                strokeWidth="1.8"
              />
              <circle cx="16" cy="16" r="2.4" fill="#fff" />
              <path d="M16 9v3M16 20v3M9 16h3M20 16h3" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" />
              <defs>
                <linearGradient id="g" x1="0" y1="0" x2="32" y2="32">
                  <stop stopColor="#4080FF" />
                  <stop offset="1" stopColor="#0FC6C2" />
                </linearGradient>
              </defs>
            </svg>
          </div>
          {!collapsed && (
            <div className="sider-brand__text">
              <span className="sider-brand__name">VOS-RS</span>
              <span className="sider-brand__sub">VoIP 运营平台</span>
            </div>
          )}
        </div>

        <nav className="sider-nav">
          {NAV_GROUPS.map((g) => (
            <div className="sider-nav__group" key={g}>
              {!collapsed && <div className="sider-nav__group-title">{g}</div>}
              {NAV_ITEMS.filter((it) => it.group === g).map((item) => (
                <div
                  key={item.key}
                  className={`sider-nav__item${
                    selectedKey === item.key ? ' is-active' : ''
                  }`}
                  onClick={() => navigate(item.key)}
                >
                  <span className="sider-nav__icon">{item.icon}</span>
                  <span className="sider-nav__label">
                    <span className="sider-nav__title">{item.title}</span>
                    {!collapsed && (
                      <span className="sider-nav__desc">{item.desc}</span>
                    )}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </nav>

        {!collapsed && (
          <div className="sider-footer">
            <div className="sider-footer__card">
              <div className="sider-footer__dot" />
              <div>
                <div className="sider-footer__title">系统运行中</div>
                <div className="sider-footer__sub">v0.1.0 · edge online</div>
              </div>
            </div>
          </div>
        )}
      </Sider>

      <ArcoLayout className="app-main">
        <Header className="app-header">
          <div className="app-header__left">
            <Button
              type="text"
              className="app-header__collapse"
              onClick={() => setCollapsed(!collapsed)}
              icon={collapsed ? <IconMenuUnfold /> : <IconMenuFold />}
            />
            <Breadcrumb className="app-header__crumb">
              <Breadcrumb.Item>控制台</Breadcrumb.Item>
              <Breadcrumb.Item>{activeNav.title}</Breadcrumb.Item>
            </Breadcrumb>
          </div>

          <div className="app-header__right">
            <div className="live-pill">
              <span className="live-pill__dot" />
              实时
            </div>
            <Tag color="blue" className="env-tag">DEV</Tag>
            <Dropdown droplist={userDropdown} trigger="hover" position="br">
              <div className="app-header__user">
                <Avatar size={30} className="app-header__avatar">
                  A
                </Avatar>
                <span className="app-header__username">Admin</span>
                <IconDown style={{ fontSize: 12, color: 'var(--text-3)' }} />
              </div>
            </Dropdown>
          </div>
        </Header>

        <Content className="app-content">{children}</Content>
      </ArcoLayout>
    </ArcoLayout>
  );
}
