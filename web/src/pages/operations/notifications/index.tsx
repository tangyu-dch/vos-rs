import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Button,
  Card,
  CardBody,
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
  Table,
  TableBody,
  TableCell,
  TableColumn,
  TableHeader,
  TableRow,
} from '@heroui/react';
import { Bell, CheckCheck, FileText, Mail, RefreshCw, Search } from 'lucide-react';
import DOMPurify from 'dompurify';
import { toast } from 'sonner';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission } from '@/services/auth';
import {
  getNotifications,
  getUnreadNotificationCount,
  markAllNotificationsRead,
  markNotificationRead,
  type NotificationItem,
} from '@/services/notifications';
import {
  getMyAnnouncements,
  markMyAnnouncementRead,
  type MyAnnouncement,
} from '@/services/announcements';

type InboxKind = 'messages' | 'announcements';
type InboxRow = {
  id: string;
  title: string;
  content: string;
  category: string;
  isRead: boolean;
  createdAt: string;
  source: InboxKind;
  raw: NotificationItem | MyAnnouncement;
};

const PAGE_SIZE = 20;
const categoryOptions = [
  { key: 'all', label: '全部类型' },
  { key: 'system', label: '系统通知' },
  { key: 'server', label: '服务异常' },
  { key: 'trunk', label: '中继异常' },
  { key: 'registration', label: '注册异常' },
  { key: 'balance', label: '余额提醒' },
  { key: 'call_quality', label: '通话质量' },
  { key: 'risk', label: '风控预警' },
  { key: 'security', label: '安全告警' },
  { key: 'maintenance', label: '维护公告' },
  { key: 'business', label: '业务公告' },
];

const categoryLabels: Record<string, string> = Object.fromEntries(
  categoryOptions.map((item) => [item.key, item.label]),
);
const plainContent = (content: string) =>
  content
    .replace(/<[^>]*>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();

function toAnnouncementRow(item: MyAnnouncement): InboxRow {
  return {
    id: item.id,
    title: item.title,
    content: item.content,
    category: item.category,
    isRead: item.is_read,
    createdAt: item.published_at ?? item.scheduled_at ?? item.created_at,
    source: 'announcements',
    raw: item,
  };
}

function toMessageRow(item: NotificationItem): InboxRow {
  return {
    id: item.id,
    title: item.title,
    content: item.message,
    category: item.category,
    isRead: item.isRead,
    createdAt: item.createdAt,
    source: 'messages',
    raw: item,
  };
}

export function NotificationsPage() {
  const { session } = useAuth();
  const canRead = Boolean(session && hasPermission(session, 'notifications.read'));
  const [kind, setKind] = useState<InboxKind>('messages');
  const [rows, setRows] = useState<InboxRow[]>([]);
  const [messageUnread, setMessageUnread] = useState(0);
  const [announcementUnread, setAnnouncementUnread] = useState(0);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('all');
  const [status, setStatus] = useState('all');
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [detail, setDetail] = useState<InboxRow | null>(null);
  const [loading, setLoading] = useState(true);
  const [updating, setUpdating] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      if (kind === 'messages') {
        const [result, unread, announcementCount] = await Promise.all([
          getNotifications(status === 'unread', page, PAGE_SIZE),
          getUnreadNotificationCount(),
          getMyAnnouncements({ unread_only: true, page: 1, page_size: 1 }),
        ]);
        setRows(result.items.map(toMessageRow));
        setTotal(result.total ?? result.items.length);
        setMessageUnread(unread);
        setAnnouncementUnread(announcementCount.total);
      } else {
        const [result, unread, messageCount] = await Promise.all([
          getMyAnnouncements({
            q: query || undefined,
            category: category === 'all' ? undefined : category,
            unread_only: status === 'unread',
            page,
            page_size: PAGE_SIZE,
          }),
          getMyAnnouncements({ unread_only: true, page: 1, page_size: 1 }),
          getUnreadNotificationCount(),
        ]);
        setRows(result.items.map(toAnnouncementRow));
        setTotal(result.total);
        setAnnouncementUnread(unread.total);
        setMessageUnread(messageCount);
      }
      setSelected(new Set());
    } catch {
      toast.error('消息列表加载失败');
    } finally {
      setLoading(false);
    }
  }, [category, kind, page, query, status]);

  useEffect(() => {
    void load();
  }, [load]);

  const visibleRows = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    return rows.filter((row) => {
      if (keyword && !`${row.title} ${row.content}`.toLowerCase().includes(keyword)) return false;
      if (category !== 'all' && row.category !== category) return false;
      if (status === 'read' && !row.isRead) return false;
      if (status === 'unread' && row.isRead) return false;
      return true;
    });
  }, [category, query, rows, status]);

  const markRowsRead = async (targets: InboxRow[]) => {
    if (targets.length === 0 || (targets.some((row) => row.source === 'messages') && !canRead))
      return;
    setUpdating(true);
    try {
      await Promise.all(
        targets
          .filter((row) => !row.isRead)
          .map((row) =>
            row.source === 'messages'
              ? markNotificationRead(row.id)
              : markMyAnnouncementRead(row.id),
          ),
      );
      toast.success('已标记为已读');
      await load();
    } catch {
      toast.error('标记已读失败');
    } finally {
      setUpdating(false);
    }
  };

  const markAllRead = async () => {
    if (kind === 'messages' && !canRead) return;
    setUpdating(true);
    try {
      if (kind === 'messages') await markAllNotificationsRead();
      else {
        for (let batch = 0; batch < 100; batch += 1) {
          const result = await getMyAnnouncements({ unread_only: true, page: 1, page_size: 100 });
          if (result.items.length === 0) break;
          await Promise.all(result.items.map((item) => markMyAnnouncementRead(item.id)));
        }
      }
      toast.success('全部消息已读');
      await load();
    } catch {
      toast.error('全部已读失败');
    } finally {
      setUpdating(false);
    }
  };

  const openDetail = async (row: InboxRow) => {
    setDetail(row);
    if (!row.isRead && (row.source === 'announcements' || canRead)) await markRowsRead([row]);
  };

  const changeKind = (next: InboxKind) => {
    setKind(next);
    setPage(1);
    setCategory('all');
    setStatus('all');
    setQuery('');
  };
  const selectedRows = rows.filter((row) => selected.has(row.id));
  const canMarkCurrent = kind === 'announcements' || canRead;

  return (
    <section className="flex flex-col gap-4">
      <header>
        <h1 className="flex items-center gap-2 text-lg font-semibold">
          <Bell className="h-5 w-5 text-primary" />
          消息通知
        </h1>
        <p className="mt-1 text-small text-default-500">统一查看运行告警、费用提醒和平台公告</p>
      </header>
      <Card shadow="none" className="overview-card overflow-hidden">
        <CardBody className="grid min-h-[620px] grid-cols-1 p-0 md:grid-cols-[220px_1fr]">
          <aside className="border-b border-default-200 bg-content2/40 p-3 md:border-b-0 md:border-r">
            <p className="px-3 pb-2 text-tiny font-semibold text-default-400">消息分类</p>
            {[
              { key: 'messages' as const, label: '我的消息', icon: Mail, count: messageUnread },
              {
                key: 'announcements' as const,
                label: '我的公告',
                icon: FileText,
                count: announcementUnread,
              },
            ].map((item) => (
              <button
                key={item.key}
                type="button"
                onClick={() => changeKind(item.key)}
                className={`mb-1 flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-small transition-colors ${kind === item.key ? 'bg-primary/10 font-semibold text-primary' : 'text-default-600 hover:bg-default-100'}`}
              >
                <item.icon className="h-4 w-4" />
                <span className="flex-1">{item.label}</span>
                {item.count > 0 && (
                  <Chip size="sm" color="danger" variant="flat">
                    {item.count > 99 ? '99+' : item.count}
                  </Chip>
                )}
              </button>
            ))}
          </aside>
          <div className="min-w-0 p-4">
            <div className="flex flex-wrap items-center gap-2 border-b border-default-200 pb-4">
              <Input
                size="sm"
                variant="bordered"
                isClearable
                className="w-full sm:w-80 sm:flex-none"
                placeholder="搜索标题或内容"
                value={query}
                onValueChange={(value) => {
                  setQuery(value);
                  setPage(1);
                }}
                startContent={<Search className="h-4 w-4 text-default-400" />}
              />
              <Select
                size="sm"
                variant="bordered"
                aria-label="消息类型"
                className="w-full sm:w-36"
                selectedKeys={[category]}
                onSelectionChange={(keys) => {
                  setCategory(String(Array.from(keys)[0] ?? 'all'));
                  setPage(1);
                }}
              >
                {categoryOptions.map((item) => (
                  <SelectItem key={item.key}>{item.label}</SelectItem>
                ))}
              </Select>
              <Select
                size="sm"
                variant="bordered"
                aria-label="阅读状态"
                className="w-full sm:w-32"
                selectedKeys={[status]}
                onSelectionChange={(keys) => {
                  setStatus(String(Array.from(keys)[0] ?? 'all'));
                  setPage(1);
                }}
              >
                <SelectItem key="all">全部状态</SelectItem>
                <SelectItem key="unread">尚未查看</SelectItem>
                <SelectItem key="read">已经查看</SelectItem>
              </Select>
              <span className="whitespace-nowrap text-small text-default-400">
                共 {total} 条记录
              </span>
              <Button
                isIconOnly
                size="sm"
                variant="flat"
                isLoading={loading}
                onPress={() => void load()}
                aria-label="刷新列表"
              >
                <RefreshCw className="h-4 w-4" />
              </Button>
              <Button
                size="sm"
                variant="flat"
                startContent={<CheckCheck className="h-4 w-4" />}
                isDisabled={!canMarkCurrent || selectedRows.length === 0}
                isLoading={updating}
                onPress={() => void markRowsRead(selectedRows)}
              >
                批量已读
              </Button>
              <Button
                size="sm"
                color="primary"
                variant="flat"
                isDisabled={
                  !canMarkCurrent ||
                  (kind === 'messages' ? messageUnread : announcementUnread) === 0
                }
                isLoading={updating}
                onPress={() => void markAllRead()}
              >
                全部已读
              </Button>
            </div>
            {loading ? (
              <div className="flex h-96 items-center justify-center">
                <Spinner label="正在加载消息" />
              </div>
            ) : (
              <Table
                aria-label={kind === 'messages' ? '我的消息列表' : '我的公告列表'}
                selectionMode="multiple"
                selectedKeys={selected}
                onSelectionChange={(keys) =>
                  setSelected(
                    keys === 'all'
                      ? new Set(visibleRows.map((row) => row.id))
                      : new Set(Array.from(keys).map(String)),
                  )
                }
                removeWrapper
                className="mt-3"
              >
                <TableHeader>
                  <TableColumn key="title">标题</TableColumn>
                  <TableColumn key="category" width={120}>
                    类型
                  </TableColumn>
                  <TableColumn key="status" width={110}>
                    状态
                  </TableColumn>
                  <TableColumn key="time" width={180}>
                    时间
                  </TableColumn>
                  <TableColumn key="action" width={90} align="end">
                    操作
                  </TableColumn>
                </TableHeader>
                <TableBody items={visibleRows} emptyContent="暂无相关消息">
                  {(row) => (
                    <TableRow key={row.id} className={row.isRead ? 'text-default-500' : ''}>
                      <TableCell>
                        <div className="max-w-xl">
                          <strong
                            className={row.isRead ? 'font-medium' : 'font-semibold text-foreground'}
                          >
                            {row.title}
                          </strong>
                          <p className="mt-1 truncate text-tiny text-default-400">
                            {plainContent(row.content)}
                          </p>
                        </div>
                      </TableCell>
                      <TableCell>
                        <Chip size="sm" variant="flat">
                          {categoryLabels[row.category] ?? '系统通知'}
                        </Chip>
                      </TableCell>
                      <TableCell>
                        {row.isRead ? (
                          <Chip size="sm" variant="flat">
                            已经查看
                          </Chip>
                        ) : (
                          <Chip size="sm" color="primary" variant="dot">
                            尚未查看
                          </Chip>
                        )}
                      </TableCell>
                      <TableCell>
                        <span className="text-tiny">
                          {new Date(row.createdAt).toLocaleString('zh-CN')}
                        </span>
                      </TableCell>
                      <TableCell>
                        <Button
                          size="sm"
                          variant="light"
                          color="primary"
                          onPress={() => void openDetail(row)}
                        >
                          查看
                        </Button>
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            )}
            {total > PAGE_SIZE && (
              <Pagination
                className="mt-4 justify-center"
                page={page}
                total={Math.ceil(total / PAGE_SIZE)}
                onChange={setPage}
                showControls
              />
            )}
          </div>
        </CardBody>
      </Card>
      <Modal isOpen={Boolean(detail)} onOpenChange={(open) => !open && setDetail(null)} size="2xl">
        <ModalContent>
          <ModalHeader>{detail?.title}</ModalHeader>
          <ModalBody>
            <div className="flex gap-2">
              <Chip size="sm" variant="flat">
                {categoryLabels[detail?.category ?? ''] ?? '系统通知'}
              </Chip>
              <span className="text-tiny text-default-400">
                {detail && new Date(detail.createdAt).toLocaleString('zh-CN')}
              </span>
            </div>
            <div
              className="prose prose-sm max-w-none text-default-700"
              dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(detail?.content ?? '') }}
            />
          </ModalBody>
          <ModalFooter>
            <Button color="primary" onPress={() => setDetail(null)}>
              我知道了
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </section>
  );
}
