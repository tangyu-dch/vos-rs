import { ReactNode, useState, useEffect } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import './Layout.css';

interface LayoutProps {
  children: ReactNode;
}

interface NavItem {
  key: string;
  icon: string;
  title: string;
  group: string;
  badge?: number;
}

const NAV_ITEMS: NavItem[] = [
  { key: '/dashboard', icon: '⊞', title: '工作台总览', group: '实时监控' },
  { key: '/active-calls', icon: '📞', title: '活跃呼叫', group: '实时监控', badge: 3 },
  { key: '/users', icon: '👥', title: 'SIP 用户', group: '号码路由' },
  { key: '/gateways', icon: '🗄', title: '落地网关', group: '号码路由' },
  { key: '/peer-gateways', icon: '🔗', title: '对接网关', group: '号码路由' },
  { key: '/routes', icon: '🔀', title: '路由管理', group: '号码路由' },
  { key: '/registrations', icon: '📋', title: '注册信息', group: '号码路由' },
  { key: '/numbers', icon: '📞', title: '号码库存', group: '号码路由' },
  { key: '/cdr', icon: '📝', title: '呼叫记录', group: '数据分析' },
  { key: '/reports', icon: '📊', title: '报表分析', group: '数据分析' },
  { key: '/rates', icon: '💰', title: '费率', group: '计费' },
  { key: '/accounts', icon: '🏦', title: '账户', group: '计费' },
  { key: '/recordings', icon: '🎙', title: '录音', group: '数据分析' },
  { key: '/anti-fraud', icon: '🛡', title: '防盗打', group: '安全' },
];

const NAV_GROUPS = ['实时监控', '号码路由', '数据分析', '计费', '安全'];

const TAB_ITEMS = [
  { key: '/dashboard', icon: '⊞', label: '工作台' },
  { key: '/active-calls', icon: '📞', label: '通话', badge: 3 },
  { key: '/users', icon: '👤', label: '客户' },
  { key: '/reports', icon: '📊', label: '报表' },
  { key: '/more', icon: '⋯', label: '更多' },
];

export default function Layout({ children }: LayoutProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [isMobile, setIsMobile] = useState(false);
  const [isTablet, setIsTablet] = useState(false);
  const location = useLocation();
  const navigate = useNavigate();

  const selectedKey =
    location.pathname === '/' ? '/dashboard' : location.pathname;

  useEffect(() => {
    const checkSize = () => {
      setIsMobile(window.innerWidth < 768);
      setIsTablet(window.innerWidth >= 768 && window.innerWidth < 1024);
    };
    checkSize();
    window.addEventListener('resize', checkSize);
    return () => window.removeEventListener('resize', checkSize);
  }, []);

  useEffect(() => {
    if (isMobile || isTablet) {
      setSidebarOpen(false);
    }
  }, [location.pathname, isMobile, isTablet]);

  const toggleSidebar = () => {
    if (isMobile || isTablet) {
      setSidebarOpen(!sidebarOpen);
    } else {
      setCollapsed(!collapsed);
    }
  };

  const closeSidebar = () => {
    setSidebarOpen(false);
  };

  return (
    <div className="app" data-theme={localStorage.getItem('vos-theme') || 'dark'}>
      {/* Sidebar */}
      <aside className={`sidebar ${collapsed ? 'sidebar--collapsed' : ''} ${sidebarOpen ? 'sidebar--open' : ''}`}>
        <div className="sidebar-brand">
          <div className="sidebar-brand__logo">
            <svg width="22" height="22" viewBox="0 0 32 32" fill="none">
              <rect width="32" height="32" rx="8" fill="url(#logo-grad)" />
              <path d="M9 11.5a3.5 3.5 0 0 1 3.5-3.5h7A3.5 3.5 0 0 1 23 11.5v9A3.5 3.5 0 0 1 19.5 24h-7A3.5 3.5 0 0 1 9 20.5v-9Z" stroke="#fff" strokeWidth="1.8" />
              <circle cx="16" cy="16" r="2.4" fill="#fff" />
              <path d="M16 9v3M16 20v3M9 16h3M20 16h3" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" />
              <defs>
                <linearGradient id="logo-grad" x1="0" y1="0" x2="32" y2="32">
                  <stop stopColor="#4080FF" />
                  <stop offset="1" stopColor="#0FC6C2" />
                </linearGradient>
              </defs>
            </svg>
          </div>
          {!collapsed && (
            <div className="sidebar-brand__text">
              <span className="sidebar-brand__name">VOS-RS</span>
              <span className="sidebar-brand__sub">VoIP 运营平台</span>
            </div>
          )}
        </div>

        <nav className="sidebar-nav">
          {NAV_GROUPS.map((g) => (
            <div className="sidebar-nav__group" key={g}>
              {!collapsed && <div className="sidebar-nav__group-title">{g}</div>}
              {NAV_ITEMS.filter((it) => it.group === g).map((item) => (
                <div
                  key={item.key}
                  className={`sidebar-nav__item ${selectedKey === item.key ? 'is-active' : ''}`}
                  onClick={() => navigate(item.key)}
                >
                  <span className="sidebar-nav__icon">{item.icon}</span>
                  {!collapsed && (
                    <span className="sidebar-nav__title">{item.title}</span>
                  )}
                  {!collapsed && item.badge && (
                    <span className="sidebar-nav__badge">{item.badge}</span>
                  )}
                </div>
              ))}
            </div>
          ))}
        </nav>

        {!collapsed && (
          <div className="sidebar-footer">
            <div className="sidebar-status">
              <span className="sidebar-status__dot" />
              <div>
                <div className="sidebar-status__title">系统运行中</div>
                <div className="sidebar-status__sub">v0.1.0 · edge online</div>
              </div>
            </div>
          </div>
        )}
      </aside>

      {/* Sidebar Overlay (mobile/tablet) */}
      {sidebarOpen && (
        <div className="sidebar-overlay" onClick={closeSidebar} />
      )}

      {/* Main Content */}
      <div className="app-main">
        {/* Topbar */}
        <header className="topbar">
          <div className="topbar-left">
            <button className="topbar-btn menu-btn" onClick={toggleSidebar}>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <line x1="3" y1="6" x2="21" y2="6" />
                <line x1="3" y1="12" x2="21" y2="12" />
                <line x1="3" y1="18" x2="21" y2="18" />
              </svg>
            </button>
            <div className="topbar-brand">
              <div className="topbar-brand__logo">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z" />
                </svg>
              </div>
              <span>VOS-RS</span>
            </div>
          </div>

          <div className="topbar-search">
            <svg className="topbar-search__icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input type="text" placeholder="搜索客户、号码、通话记录..." />
          </div>

          <div className="topbar-right">
            <button className="topbar-btn" title="通知">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
                <path d="M13.73 21a2 2 0 0 1-3.46 0" />
              </svg>
              <span className="topbar-btn__badge" />
            </button>

            <button className="theme-toggle" onClick={toggleTheme} title="切换主题">
              <span className="theme-toggle__knob" id="themeKnob">
                <span id="themeIcon">🌙</span>
              </span>
            </button>

            <div className="topbar-avatar">A</div>
          </div>
        </header>

        {/* Content */}
        <main className="app-content">{children}</main>
      </div>

      {/* Mobile Bottom Tab Bar */}
      {isMobile && (
        <nav className="bottombar">
          <div className="bottombar-inner">
            {TAB_ITEMS.map((tab) => (
              <button
                key={tab.key}
                className={`tab-item ${selectedKey === tab.key || (tab.key === '/more' && sidebarOpen) ? 'tab-item--active' : ''}`}
                onClick={() => {
                  if (tab.key === '/more') {
                    toggleSidebar();
                  } else {
                    navigate(tab.key);
                  }
                }}
              >
                <span className="tab-item__icon">{tab.icon}</span>
                <span className="tab-item__label">{tab.label}</span>
                {tab.badge && <span className="tab-item__badge">{tab.badge}</span>}
              </button>
            ))}
          </div>
        </nav>
      )}

      {/* Mobile FAB */}
      {isMobile && (
        <button className="fab" onClick={() => navigate('/active-calls')}>
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
            <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z" />
          </svg>
        </button>
      )}
    </div>
  );
}

function toggleTheme() {
  const html = document.documentElement;
  const icon = document.getElementById('themeIcon');
  const current = html.getAttribute('data-theme');
  const next = current === 'dark' ? 'light' : 'dark';
  html.setAttribute('data-theme', next);
  document.querySelector('.app')?.setAttribute('data-theme', next);
  if (icon) icon.textContent = next === 'dark' ? '🌙' : '☀️';
  localStorage.setItem('vos-theme', next);
}

// Load saved theme
(function initTheme() {
  const saved = localStorage.getItem('vos-theme');
  if (saved) {
    document.documentElement.setAttribute('data-theme', saved);
    document.querySelector('.app')?.setAttribute('data-theme', saved);
    const icon = document.getElementById('themeIcon');
    if (icon) icon.textContent = saved === 'dark' ? '🌙' : '☀️';
  }
})();
