import { useCallback, useEffect, useState, type ReactNode } from 'react';
import {
  Badge,
  Button,
  Chip,
  Divider,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Spinner,
  Tooltip,
} from '@heroui/react';
import {
  Activity,
  Bell,
  CheckCheck,
  ChevronRight,
  CircleAlert,
  Info,
  LockKeyhole,
  PhoneOff,
  RefreshCw,
  Server,
  ShieldAlert,
  UserX,
  WalletCards,
} from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission } from '@/services/auth';
import {
  getNotifications,
  getUnreadNotificationCount,
  markAllNotificationsRead,
  markNotificationRead,
  type NotificationCategory,
  type NotificationItem,
  type NotificationSeverity,
} from '@/services/notifications';

const REFRESH_INTERVAL_MS = 30_000;

const CATEGORY_META: Record<NotificationCategory, { label: string; icon: ReactNode }> = {
  server: { label: '服务异常', icon: <Server className="h-4 w-4" /> },
  trunk: { label: '中继异常', icon: <PhoneOff className="h-4 w-4" /> },
  registration: { label: '注册异常', icon: <UserX className="h-4 w-4" /> },
  balance: { label: '余额不足', icon: <WalletCards className="h-4 w-4" /> },
  call_quality: { label: '通话质量', icon: <Activity className="h-4 w-4" /> },
  risk: { label: '风控预警', icon: <ShieldAlert className="h-4 w-4" /> },
  security: { label: '安全告警', icon: <LockKeyhole className="h-4 w-4" /> },
  system: { label: '系统通知', icon: <Info className="h-4 w-4" /> },
};

const SEVERITY_META: Record<
  NotificationSeverity,
  {
    label: string;
    color: 'danger' | 'warning' | 'primary' | 'success';
    iconClassName: string;
  }
> = {
  critical: { label: '紧急', color: 'danger', iconClassName: 'bg-danger/15 text-danger' },
  warning: { label: '重要', color: 'warning', iconClassName: 'bg-warning/15 text-warning' },
  info: { label: '提醒', color: 'primary', iconClassName: 'bg-primary/15 text-primary' },
  success: { label: '恢复', color: 'success', iconClassName: 'bg-success/15 text-success' },
};

function formatNotificationTime(value: string): string {
  const timestamp = new Date(value).getTime();
  if (!Number.isFinite(timestamp)) return '时间未知';
  const elapsed = Math.max(0, Date.now() - timestamp);
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 1) return '刚刚';
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days} 天前`;
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(timestamp);
}

function NotificationRow({
  item,
  onRead,
  onNavigate,
}: {
  item: NotificationItem;
  onRead: (id: string) => Promise<void>;
  onNavigate: (item: NotificationItem) => Promise<void>;
}) {
  const category = CATEGORY_META[item.category];
  const severity = SEVERITY_META[item.severity];
  return (
    <button
      type="button"
      className={`w-full px-4 py-3 text-left transition-colors hover:bg-default-100 ${item.isRead ? 'opacity-70' : 'bg-primary/5'}`}
      onClick={() => void (item.actionUrl ? onNavigate(item) : onRead(item.id))}
      aria-label={`${item.isRead ? '已读' : '未读'}消息：${item.title}`}
    >
      <div className="flex items-start gap-3">
        <span
          className={`mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full ${severity.iconClassName}`}
        >
          {category.icon}
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            {!item.isRead && (
              <span className="h-2 w-2 shrink-0 rounded-full bg-primary" aria-hidden />
            )}
            <strong className="truncate text-small text-foreground">{item.title}</strong>
            <Chip
              size="sm"
              variant="flat"
              color={severity.color}
              className="h-5 shrink-0 text-[10px]"
            >
              {severity.label}
            </Chip>
          </span>
          <span className="mt-1 line-clamp-2 text-tiny leading-5 text-default-500">
            {item.message}
          </span>
          <span className="mt-1.5 flex items-center justify-between gap-2 text-[11px] text-default-400">
            <span>{category.label}</span>
            <span>{formatNotificationTime(item.createdAt)}</span>
          </span>
        </span>
      </div>
    </button>
  );
}

export function NotificationCenter() {
  const navigate = useNavigate();
  const { session } = useAuth();
  const canMarkRead = Boolean(session && hasPermission(session, 'notifications.read'));
  const [isOpen, setIsOpen] = useState(false);
  const [items, setItems] = useState<NotificationItem[]>([]);
  const [unreadCount, setUnreadCount] = useState(0);
  const [loading, setLoading] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [error, setError] = useState('');

  const refreshCount = useCallback(async () => {
    try {
      setUnreadCount(await getUnreadNotificationCount());
    } catch {
      // 顶栏轮询失败时保留上次结果，避免短暂网络抖动清空未读数。
    }
  }, []);

  const refreshList = useCallback(async (quiet = false) => {
    if (!quiet) setLoading(true);
    try {
      const result = await getNotifications(false, 1, 5);
      setItems(result.items);
      setError('');
    } catch {
      setError('消息加载失败，请稍后重试');
    } finally {
      if (!quiet) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshCount();
    const timer = window.setInterval(() => {
      if (document.visibilityState === 'visible') {
        void refreshCount();
        if (isOpen) void refreshList(true);
      }
    }, REFRESH_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [isOpen, refreshCount, refreshList]);

  useEffect(() => {
    if (isOpen) void refreshList();
  }, [isOpen, refreshList]);

  const handleRead = async (id: string) => {
    const target = items.find((item) => item.id === id);
    if (!target || target.isRead || !canMarkRead) return;
    setItems((current) =>
      current.map((item) => (item.id === id ? { ...item, isRead: true } : item)),
    );
    setUnreadCount((count) => Math.max(0, count - 1));
    try {
      await markNotificationRead(id);
    } catch {
      setItems((current) =>
        current.map((item) => (item.id === id ? { ...item, isRead: false } : item)),
      );
      setUnreadCount((count) => count + 1);
      setError('标记已读失败，请重试');
    }
  };

  const handleNavigate = async (item: NotificationItem) => {
    await handleRead(item.id);
    if (item.actionUrl?.startsWith('/')) {
      setIsOpen(false);
      navigate(item.actionUrl);
    }
  };

  const handleReadAll = async () => {
    setUpdating(true);
    try {
      await markAllNotificationsRead();
      setItems((current) => current.map((item) => ({ ...item, isRead: true })));
      setUnreadCount(0);
      setError('');
    } catch {
      setError('全部已读操作失败，请重试');
    } finally {
      setUpdating(false);
    }
  };

  const badgeContent = unreadCount > 99 ? '99+' : String(unreadCount);
  return (
    <Popover placement="bottom-end" offset={10} isOpen={isOpen} onOpenChange={setIsOpen}>
      <PopoverTrigger>
        <Button
          isIconOnly
          variant="light"
          size="sm"
          aria-label={unreadCount > 0 ? `消息通知，${unreadCount} 条未读` : '消息通知'}
        >
          <Badge
            content={badgeContent}
            color="danger"
            size="sm"
            isInvisible={unreadCount === 0}
            shape="circle"
          >
            <Bell className="h-5 w-5" />
          </Badge>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[min(92vw,380px)] overflow-hidden border border-default-200 bg-content1 p-0 shadow-lg">
        <section className="w-full" aria-label="消息中心">
          <div className="flex items-center justify-between gap-3 px-4 py-3">
            <div>
              <h2 className="text-base font-semibold text-foreground">通知</h2>
              <p className="mt-0.5 text-tiny text-default-400">
                {unreadCount > 0 ? `${unreadCount} 条消息待查看` : '暂无未读消息'}
              </p>
            </div>
            <Tooltip content="刷新通知">
              <Button
                isIconOnly
                size="sm"
                variant="light"
                onPress={() => void Promise.all([refreshList(), refreshCount()])}
                aria-label="刷新通知"
              >
                <RefreshCw className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
              </Button>
            </Tooltip>
          </div>
          <Divider />
          <div className="max-h-[min(56vh,400px)] overflow-y-auto">
            {loading && items.length === 0 ? (
              <div className="flex h-40 items-center justify-center">
                <Spinner label="正在加载消息" size="sm" />
              </div>
            ) : error && items.length === 0 ? (
              <div className="flex h-40 flex-col items-center justify-center gap-2 px-6 text-center text-small text-danger">
                <CircleAlert className="h-6 w-6" />
                <span>{error}</span>
              </div>
            ) : items.length === 0 ? (
              <div className="flex h-40 flex-col items-center justify-center gap-2 text-default-400">
                <Bell className="h-8 w-8" />
                <span className="text-small">暂无通知</span>
              </div>
            ) : (
              items.map((item, index) => (
                <div key={item.id}>
                  {index > 0 && <Divider />}
                  <NotificationRow item={item} onRead={handleRead} onNavigate={handleNavigate} />
                </div>
              ))
            )}
          </div>
          {error && items.length > 0 && (
            <p className="border-t border-default-200 px-4 py-2 text-tiny text-danger">{error}</p>
          )}
          <div className="flex items-center justify-between border-t border-default-200 px-2 py-1.5">
            <Button
              variant="light"
              color="primary"
              endContent={<ChevronRight className="h-4 w-4" />}
              onPress={() => {
                setIsOpen(false);
                navigate('/notifications');
              }}
            >
              查看更多
            </Button>
            <Button
              variant="light"
              color="primary"
              startContent={<CheckCheck className="h-4 w-4" />}
              isDisabled={!canMarkRead || unreadCount === 0}
              isLoading={updating}
              onPress={() => void handleReadAll()}
            >
              全部已读
            </Button>
          </div>
        </section>
      </PopoverContent>
    </Popover>
  );
}
