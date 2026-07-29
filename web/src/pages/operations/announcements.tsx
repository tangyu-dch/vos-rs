import { useCallback, useEffect, useState } from 'react';
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
  Table,
  TableBody,
  TableCell,
  TableColumn,
  TableHeader,
  TableRow,
  Tooltip,
} from '@heroui/react';
import { Megaphone, Pencil, Plus, RefreshCw, Rocket, Search, Trash2 } from 'lucide-react';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission } from '@/services/auth';
import {
  createAnnouncement,
  deleteAnnouncement,
  getAnnouncements,
  publishAnnouncement,
  updateAnnouncement,
  type Announcement,
  type AnnouncementInput,
} from '@/services/announcements';
import { message } from '@/utils/toast';
import { AnnouncementForm } from './announcement-form';

const PAGE_SIZE = 20;
const categoryLabels: Record<string, string> = {
  system: '系统公告',
  maintenance: '维护公告',
  business: '业务公告',
  security: '安全公告',
};
const statusMeta: Record<
  string,
  { label: string; color: 'default' | 'primary' | 'success' | 'warning' }
> = {
  draft: { label: '草稿', color: 'default' },
  published: { label: '已发布', color: 'success' },
};
const plainContent = (content: string) =>
  content
    .replace(/<[^>]*>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();

export function AnnouncementsPage() {
  const { session } = useAuth();
  const may = (permission: string) => Boolean(session && hasPermission(session, permission));
  const canCreate = may('announcements.create');
  const canUpdate = may('announcements.update');
  const canDelete = may('announcements.delete');
  const canPublish = may('announcements.publish');
  const [items, setItems] = useState<Announcement[]>([]);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('all');
  const [status, setStatus] = useState('all');
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Announcement | null>(null);
  const [deleting, setDeleting] = useState<Announcement | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const result = await getAnnouncements({
        q: query || undefined,
        category: category === 'all' ? undefined : category,
        status: status === 'all' ? undefined : status,
        page,
        page_size: PAGE_SIZE,
      });
      const keyword = query.trim().toLowerCase();
      setItems(
        keyword
          ? result.items.filter((item) =>
              `${item.title} ${item.content}`.toLowerCase().includes(keyword),
            )
          : result.items,
      );
      setTotal(result.total);
    } catch (error) {
      message.error(error instanceof Error ? error.message : '公告列表加载失败');
    } finally {
      setLoading(false);
    }
  }, [category, page, query, status]);

  useEffect(() => {
    void load();
  }, [load]);

  const openCreate = () => {
    setEditing(null);
    setFormOpen(true);
  };
  const openEdit = (item: Announcement) => {
    setEditing(item);
    setFormOpen(true);
  };

  const save = async (input: AnnouncementInput, publish: boolean) => {
    setSaving(true);
    try {
      const result = editing
        ? await updateAnnouncement(editing.id, input)
        : await createAnnouncement(input);
      if (publish) await publishAnnouncement(result.id);
      message.success(
        publish ? (input.scheduled_at ? '公告已安排定时发布' : '公告已发布') : '公告草稿已保存',
      );
      setFormOpen(false);
      await load();
    } catch (error) {
      message.error(error instanceof Error ? error.message : '公告保存失败');
    } finally {
      setSaving(false);
    }
  };

  const publish = async (item: Announcement) => {
    setSaving(true);
    try {
      await publishAnnouncement(item.id);
      message.success(item.scheduled_at ? '公告已安排定时发布' : '公告已发布');
      await load();
    } catch (error) {
      message.error(error instanceof Error ? error.message : '公告发布失败');
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!deleting) return;
    setSaving(true);
    try {
      await deleteAnnouncement(deleting.id);
      message.success('公告已删除');
      setDeleting(null);
      await load();
    } catch (error) {
      message.error(error instanceof Error ? error.message : '公告删除失败');
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="flex flex-col gap-4">
      <Card shadow="none" className="overview-card">
        <CardBody className="flex flex-row flex-wrap items-center justify-between gap-4 p-5">
          <div>
            <h1 className="flex items-center gap-2 text-lg font-semibold">
              <Megaphone className="h-5 w-5 text-primary" />
              公告管理
            </h1>
            <p className="mt-1 text-small text-default-500">
              创建平台公告，配置通知范围、发布计划与送达方式
            </p>
          </div>
          <div className="flex gap-2">
            <Button
              isIconOnly
              variant="flat"
              isLoading={loading}
              onPress={() => void load()}
              aria-label="刷新公告"
            >
              <RefreshCw className="h-4 w-4" />
            </Button>
            <Button
              color="primary"
              isDisabled={!canCreate}
              startContent={<Plus className="h-4 w-4" />}
              onPress={openCreate}
            >
              新增公告
            </Button>
          </div>
        </CardBody>
      </Card>
      <Card shadow="none" className="overview-card">
        <CardBody className="gap-4 p-5">
          <div className="flex flex-wrap items-center gap-3">
            <Input
              size="sm"
              variant="bordered"
              isClearable
              className="w-full sm:w-80 sm:flex-none"
              placeholder="搜索公告标题或内容"
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
              aria-label="公告分类"
              className="w-full sm:w-36"
              selectedKeys={[category]}
              onSelectionChange={(keys) => {
                setCategory(String(Array.from(keys)[0] ?? 'all'));
                setPage(1);
              }}
            >
              <SelectItem key="all">全部分类</SelectItem>
              <SelectItem key="system">系统公告</SelectItem>
              <SelectItem key="maintenance">维护公告</SelectItem>
              <SelectItem key="business">业务公告</SelectItem>
              <SelectItem key="security">安全公告</SelectItem>
            </Select>
            <Select
              size="sm"
              variant="bordered"
              aria-label="公告状态"
              className="w-full sm:w-36"
              selectedKeys={[status]}
              onSelectionChange={(keys) => {
                setStatus(String(Array.from(keys)[0] ?? 'all'));
                setPage(1);
              }}
            >
              <SelectItem key="all">全部状态</SelectItem>
              <SelectItem key="draft">草稿</SelectItem>
              <SelectItem key="published">已发布</SelectItem>
            </Select>
            <span className="whitespace-nowrap text-small text-default-400">共 {total} 条记录</span>
          </div>
          <Table aria-label="公告管理列表" isStriped removeWrapper>
            <TableHeader>
              <TableColumn key="title">公告标题</TableColumn>
              <TableColumn key="category" width={120}>
                分类
              </TableColumn>
              <TableColumn key="audience" width={130}>
                通知范围
              </TableColumn>
              <TableColumn key="delivery" width={150}>
                通知方式
              </TableColumn>
              <TableColumn key="status" width={110}>
                状态
              </TableColumn>
              <TableColumn key="time" width={180}>
                发布时间
              </TableColumn>
              <TableColumn key="actions" width={140} align="end">
                操作
              </TableColumn>
            </TableHeader>
            <TableBody
              items={items}
              isLoading={loading}
              loadingContent="正在加载公告"
              emptyContent="暂无公告数据"
            >
              {(item) => {
                const meta = statusMeta[item.status] ?? statusMeta.draft;
                return (
                  <TableRow key={item.id}>
                    <TableCell>
                      <div className="max-w-lg">
                        <div className="flex items-center gap-2">
                          <strong>{item.title}</strong>
                          {item.pinned && (
                            <Chip size="sm" color="primary" variant="flat">
                              置顶
                            </Chip>
                          )}
                        </div>
                        <p className="mt-1 truncate text-tiny text-default-400">
                          {plainContent(item.content)}
                        </p>
                      </div>
                    </TableCell>
                    <TableCell>{categoryLabels[item.category] ?? '系统公告'}</TableCell>
                    <TableCell>
                      {item.audience === 'all'
                        ? '所有用户'
                        : `${item.audience_users?.length ?? 0} 位用户`}
                    </TableCell>
                    <TableCell>
                      {(item.delivery_methods ?? [])
                        .map((method) => (method === 'popup' ? '登录弹窗' : '系统消息'))
                        .join('、')}
                    </TableCell>
                    <TableCell>
                      <Chip size="sm" color={meta.color} variant="flat">
                        {meta.label}
                      </Chip>
                    </TableCell>
                    <TableCell>
                      <span className="text-tiny">
                        {item.published_at || item.scheduled_at
                          ? new Date(item.published_at ?? item.scheduled_at ?? '').toLocaleString(
                              'zh-CN',
                            )
                          : '尚未发布'}
                      </span>
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Tooltip content="编辑公告">
                          <span>
                            <Button
                              isIconOnly
                              size="sm"
                              variant="light"
                              isDisabled={!canUpdate}
                              onPress={() => openEdit(item)}
                              aria-label="编辑公告"
                            >
                              <Pencil className="h-4 w-4" />
                            </Button>
                          </span>
                        </Tooltip>
                        {item.status !== 'published' && (
                          <Tooltip content="发布公告">
                            <span>
                              <Button
                                isIconOnly
                                size="sm"
                                color="primary"
                                variant="light"
                                isDisabled={!canPublish}
                                isLoading={saving}
                                onPress={() => void publish(item)}
                                aria-label="发布公告"
                              >
                                <Rocket className="h-4 w-4" />
                              </Button>
                            </span>
                          </Tooltip>
                        )}
                        <Tooltip content="删除公告">
                          <span>
                            <Button
                              isIconOnly
                              size="sm"
                              color="danger"
                              variant="light"
                              isDisabled={!canDelete}
                              onPress={() => setDeleting(item)}
                              aria-label="删除公告"
                            >
                              <Trash2 className="h-4 w-4" />
                            </Button>
                          </span>
                        </Tooltip>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              }}
            </TableBody>
          </Table>
          {total > PAGE_SIZE && (
            <Pagination
              className="self-center"
              page={page}
              total={Math.ceil(total / PAGE_SIZE)}
              onChange={setPage}
              showControls
            />
          )}
        </CardBody>
      </Card>
      <AnnouncementForm
        open={formOpen}
        announcement={editing}
        canSave={editing ? canUpdate : canCreate}
        canPublish={canPublish}
        saving={saving}
        onClose={() => setFormOpen(false)}
        onSubmit={save}
      />
      <Modal
        isOpen={Boolean(deleting)}
        onOpenChange={(open) => !open && setDeleting(null)}
        size="sm"
      >
        <ModalContent>
          <ModalHeader>删除公告</ModalHeader>
          <ModalBody>确定删除“{deleting?.title}”吗？删除后无法恢复。</ModalBody>
          <ModalFooter>
            <Button variant="flat" onPress={() => setDeleting(null)}>
              取消
            </Button>
            <Button color="danger" isLoading={saving} onPress={() => void remove()}>
              确认删除
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </section>
  );
}
