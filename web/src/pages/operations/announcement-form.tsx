import { useEffect, useState } from 'react';
import {
  Button,
  Input,
  Modal,
  ModalBody,
  ModalContent,
  ModalFooter,
  ModalHeader,
  Select,
  SelectItem,
  Switch,
} from '@heroui/react';
import type {
  Announcement,
  AnnouncementCategory,
  AnnouncementDelivery,
  AnnouncementInput,
  AnnouncementTarget,
} from '@/services/announcements';
import { AiEditorField } from '@/components/ai-editor-field';

const EMPTY_INPUT: AnnouncementInput = {
  title: '',
  category: 'system',
  audience: 'all',
  audience_users: [],
  delivery_methods: ['system'],
  scheduled_at: null,
  pinned: false,
  content: '',
};

export function AnnouncementForm({
  open,
  announcement,
  canSave,
  canPublish,
  saving,
  onClose,
  onSubmit,
}: {
  open: boolean;
  announcement: Announcement | null;
  canSave: boolean;
  canPublish: boolean;
  saving: boolean;
  onClose: () => void;
  onSubmit: (input: AnnouncementInput, publish: boolean) => Promise<void>;
}) {
  const [form, setForm] = useState<AnnouncementInput>(EMPTY_INPUT);
  const [scheduled, setScheduled] = useState(false);
  const [usersText, setUsersText] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    if (!open) return;
    if (announcement) {
      setForm({
        title: announcement.title,
        category: announcement.category,
        audience: announcement.audience,
        audience_users: announcement.audience_users ?? [],
        delivery_methods: announcement.delivery_methods ?? ['system'],
        scheduled_at: announcement.scheduled_at,
        pinned: announcement.pinned,
        content: announcement.content,
      });
      setUsersText((announcement.audience_users ?? []).join(', '));
      setScheduled(Boolean(announcement.scheduled_at));
    } else {
      setForm(EMPTY_INPUT);
      setUsersText('');
      setScheduled(false);
    }
    setError('');
  }, [announcement, open]);

  const submit = async (publish: boolean) => {
    const audienceUsers = usersText
      .split(/[,，\s]+/)
      .map((value) => value.trim())
      .filter(Boolean);
    if (!form.title.trim() || !form.content.trim()) {
      setError('请填写公告标题和公告内容');
      return;
    }
    if (form.audience === 'specified' && audienceUsers.length === 0) {
      setError('请填写至少一个指定用户');
      return;
    }
    if (form.delivery_methods.length === 0) {
      setError('请选择至少一种通知方式');
      return;
    }
    if (scheduled && !form.scheduled_at) {
      setError('请选择定时发布时间');
      return;
    }
    setError('');
    await onSubmit(
      {
        ...form,
        title: form.title.trim(),
        content: form.content.trim(),
        audience_users: form.audience === 'specified' ? audienceUsers : [],
        scheduled_at: scheduled ? form.scheduled_at : null,
      },
      publish,
    );
  };

  return (
    <Modal
      isOpen={open}
      onOpenChange={(value) => !value && onClose()}
      size="2xl"
      scrollBehavior="inside"
      classNames={{
        base: 'max-h-[calc(100dvh-2rem)]',
        header: 'px-5 py-4 text-base font-semibold border-b border-divider',
        body: 'px-5 py-4 overflow-y-auto',
        footer: 'px-5 py-3 border-t border-divider',
      }}
    >
      <ModalContent>
        <ModalHeader>{announcement ? '编辑公告' : '新增公告'}</ModalHeader>
        <ModalBody className="grid grid-cols-1 gap-3 md:grid-cols-2">
          <Input
            size="sm"
            variant="bordered"
            labelPlacement="outside"
            className="md:col-span-2"
            label="公告标题"
            value={form.title}
            onValueChange={(title) => setForm((current) => ({ ...current, title }))}
            isRequired
          />
          <Select
            size="sm"
            variant="bordered"
            labelPlacement="outside"
            label="公告分类"
            selectedKeys={[form.category]}
            onSelectionChange={(keys) =>
              setForm((current) => ({
                ...current,
                category: String(Array.from(keys)[0] ?? 'system') as AnnouncementCategory,
              }))
            }
          >
            <SelectItem key="system">系统公告</SelectItem>
            <SelectItem key="maintenance">维护公告</SelectItem>
            <SelectItem key="business">业务公告</SelectItem>
            <SelectItem key="security">安全公告</SelectItem>
          </Select>
          <Select
            size="sm"
            variant="bordered"
            labelPlacement="outside"
            label="通知范围"
            selectedKeys={[form.audience]}
            onSelectionChange={(keys) =>
              setForm((current) => ({
                ...current,
                audience: String(Array.from(keys)[0] ?? 'all') as AnnouncementTarget,
              }))
            }
          >
            <SelectItem key="all">所有用户</SelectItem>
            <SelectItem key="specified">指定用户</SelectItem>
          </Select>
          {form.audience === 'specified' && (
            <Input
              size="sm"
              variant="bordered"
              labelPlacement="outside"
              className="md:col-span-2"
              label="指定用户"
              description="多个登录账号使用逗号或空格分隔"
              value={usersText}
              onValueChange={setUsersText}
            />
          )}
          <Select
            size="sm"
            variant="bordered"
            labelPlacement="outside"
            className="md:col-span-2"
            label="通知方式"
            selectionMode="multiple"
            selectedKeys={new Set(form.delivery_methods)}
            onSelectionChange={(keys) =>
              setForm((current) => ({
                ...current,
                delivery_methods: Array.from(keys).map(String) as AnnouncementDelivery[],
              }))
            }
          >
            <SelectItem key="system">系统消息</SelectItem>
            <SelectItem key="popup">登录弹窗</SelectItem>
          </Select>
          <div className="flex min-h-10 items-center rounded-lg border border-default-200 px-3">
            <Switch size="sm" color="primary" isSelected={scheduled} onValueChange={setScheduled}>
              定时发布
            </Switch>
          </div>
          <Input
            size="sm"
            variant="bordered"
            labelPlacement="outside-left"
            label="发布时间"
            type="datetime-local"
            isDisabled={!scheduled}
            value={form.scheduled_at ? form.scheduled_at.slice(0, 16) : ''}
            onValueChange={(value) =>
              setForm((current) => ({
                ...current,
                scheduled_at: value ? new Date(value).toISOString() : null,
              }))
            }
          />
          <div className="flex min-h-10 items-center rounded-lg border border-default-200 px-3 md:col-span-2">
            <Switch
              size="sm"
              color="primary"
              isSelected={form.pinned}
              onValueChange={(pinned) => setForm((current) => ({ ...current, pinned }))}
            >
              公告置顶
            </Switch>
          </div>
          <div className="md:col-span-2">
            <p className="mb-1.5 text-small font-medium text-default-700">
              公告内容<span className="ml-0.5 text-danger">*</span>
            </p>
            <AiEditorField
              value={form.content}
              onChange={(content) => setForm((current) => ({ ...current, content }))}
            />
          </div>
          {error && <p className="text-small text-danger md:col-span-2">{error}</p>}
        </ModalBody>
        <ModalFooter>
          <Button variant="flat" onPress={onClose}>
            取消
          </Button>
          <Button
            variant="flat"
            color="primary"
            isDisabled={!canSave}
            isLoading={saving}
            onPress={() => void submit(false)}
          >
            保存草稿
          </Button>
          <Button
            color="primary"
            isDisabled={!canSave || !canPublish}
            isLoading={saving}
            onPress={() => void submit(true)}
          >
            {scheduled ? '定时发布' : '立即发布'}
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
