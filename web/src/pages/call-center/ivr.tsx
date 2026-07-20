import { useEffect, useState } from 'react';
import {
  Card, CardBody, Button, Chip, Input, Modal, ModalBody, ModalHeader, ModalContent, ModalFooter,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Select, SelectItem,
  useDisclosure,
} from '@heroui/react';
import { Plus, RefreshCw, Pencil, Trash2, Volume2, Search } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState, LoadingState } from '@/components/detail-shell';
import { message } from '@/utils/toast';

interface IvrMapping {
  _uid: string;
  key: string;
  action: string;
  target: string;
  waiting_prompt: string;
  webhook_method: string;
}

interface IvrForm {
  id: string;
  name: string;
  welcome_file: string;
  timeout_sec: number;
  menu_actions: IvrMapping[];
}

const emptyForm: IvrForm = {
  id: '',
  name: '',
  welcome_file: '',
  timeout_sec: 10,
  menu_actions: [],
};

const dtmfOptions = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '*', '#', 'timeout', 'invalid'];

const actionOptions = [
  { value: 'extension', label: '分机 (extension)' },
  { value: 'pstn', label: '外呼 (pstn)' },
  { value: 'queue', label: '队列 (queue)' },
  { value: 'menu', label: '菜单 (menu)' },
  { value: 'webhook', label: '第三方 Webhook (webhook)' },
  { value: 'say', label: '语音朗读 (say)' },
  { value: 'collect_digits', label: '按键收集 (collect_digits)' },
  { value: 'voicemail', label: '语音留言 (voicemail)' },
  { value: 'hangup', label: '挂断 (hangup)' },
];

const genId = () => Math.random().toString(36).substring(2);

const newMapping = (): IvrMapping => ({
  _uid: genId(),
  key: '',
  action: '',
  target: '',
  waiting_prompt: '',
  webhook_method: 'POST',
});

export default function IvrPage() {
  const [data, setData] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [searchKey, setSearchKey] = useState('');
  const [form, setForm] = useState<IvrForm>(emptyForm);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState(false);
  const { isOpen, onOpen, onClose } = useDisclosure();

  const loadData = async () => {
    setLoading(true);
    setError('');
    try {
      const res: any = await api.get('/ivr/menus');
      setData(res.items || res.data || []);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载 IVR 列表失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, []);

  const validate = (): boolean => {
    const next: Record<string, string> = {};
    if (!form.id) next.id = '请填写菜单 ID';
    if (!form.name) next.name = '请填写菜单名称';
    setErrors(next);
    return Object.keys(next).length === 0;
  };

  const handleSave = async () => {
    if (!validate()) return;
    const payload = {
      id: form.id,
      name: form.name,
      welcome_prompt: form.welcome_file || '',
      timeout_secs: form.timeout_sec || 10,
      mappings: (form.menu_actions || []).map((a) => ({
        dtmf_key: String(a.key || ''),
        action_type: String(a.action || ''),
        action_target: String(a.target || ''),
        waiting_prompt: a.waiting_prompt || '',
        webhook_method: a.webhook_method || 'POST',
      })),
    };
    try {
      if (editing) {
        await api.put(`/ivr/menus/${form.id}`, payload);
        message.success('更新成功');
      } else {
        await api.post('/ivr/menus', payload);
        message.success('创建成功');
      }
      onClose();
      loadData();
    } catch (err: any) {
      if (err?.isAxiosError || err?.message) {
        message.error(err?.message || '保存失败');
      }
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await api.delete(`/ivr/menus/${id}`);
      message.success('删除成功');
      loadData();
    } catch (_err) {
      message.error('删除失败');
    }
  };

  const openCreate = () => {
    setForm({ ...emptyForm, menu_actions: [] });
    setErrors({});
    setEditing(false);
    onOpen();
  };

  const openEdit = (record: any) => {
    setForm({
      id: record.id,
      name: record.name,
      welcome_file: record.welcome_prompt || record.welcome_file || '',
      timeout_sec: record.timeout_secs || record.timeout_sec || 10,
      menu_actions: (record.mappings || []).map((m: any) => ({
        _uid: genId(),
        key: m.dtmf_key,
        action: m.action_type,
        target: m.action_target,
        waiting_prompt: m.waiting_prompt || '',
        webhook_method: m.webhook_method || 'POST',
      })),
    });
    setErrors({});
    setEditing(true);
    onOpen();
  };

  const updateMapping = (idx: number, patch: Partial<IvrMapping>) => {
    setForm((curr) => ({
      ...curr,
      menu_actions: curr.menu_actions.map((item, i) => (i === idx ? { ...item, ...patch } : item)),
    }));
  };

  const removeMapping = (idx: number) => {
    setForm((curr) => ({
      ...curr,
      menu_actions: curr.menu_actions.filter((_, i) => i !== idx),
    }));
  };

  const addMapping = () => {
    setForm((curr) => ({
      ...curr,
      menu_actions: [...curr.menu_actions, newMapping()],
    }));
  };

  const totalMenus = data.length;
  const totalRules = data.reduce((sum, item) => sum + (Array.isArray(item.mappings) ? item.mappings.length : 0), 0);
  const avgTimeout = data.length
    ? Math.round(data.reduce((sum, item) => sum + (item.timeout_secs || item.timeout_sec || 10), 0) / data.length)
    : 0;

  const kpis: Array<{ label: string; value: number | string; className: string }> = [
    { label: '菜单总数', value: totalMenus, className: 'text-primary' },
    { label: '按键规则总数', value: totalRules, className: 'text-secondary' },
    { label: '默认超时秒数', value: `${avgTimeout}s`, className: 'text-success' },
    { label: '平均规则/菜单', value: totalMenus ? Math.round(totalRules / totalMenus) : 0, className: 'text-default-500' },
  ];

  const filteredData = data.filter(
    (item) =>
      (item.id || '').toLowerCase().includes(searchKey.toLowerCase()) ||
      (item.name || '').toLowerCase().includes(searchKey.toLowerCase())
  );

  return (
    <div className="flex flex-col gap-5">
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        {kpis.map((kpi) => (
          <Card key={kpi.label}>
            <CardBody className="p-4">
              <div className="text-tiny font-medium text-default-500 mb-1">{kpi.label}</div>
              <div className={`text-3xl font-bold ${kpi.className}`}>{kpi.value}</div>
            </CardBody>
          </Card>
        ))}
      </div>

      {error ? (
        <ErrorState error={error} retry={loadData} />
      ) : loading && data.length === 0 ? (
        <LoadingState />
      ) : (
      <Card>
        <CardBody className="gap-4 p-5">
          <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-default-200">
            <Input
              placeholder="搜索菜单 ID / 名称"
              className="w-64"
              size="sm"
              variant="bordered"
              startContent={<Search className="w-4 h-4 text-default-400" />}
              value={searchKey}
              onValueChange={setSearchKey}
              isClearable
            />
            <div className="flex items-center gap-2">
              <Button
                variant="flat"
                size="sm"
                isLoading={loading}
                onPress={loadData}
                startContent={<RefreshCw className="w-4 h-4" />}
              >
                刷新
              </Button>
              <Button
                color="primary"
                size="sm"
                onPress={openCreate}
                startContent={<Plus className="w-4 h-4" />}
              >
                新建菜单
              </Button>
            </div>
          </div>

          <Table aria-label="IVR 菜单列表">
            <TableHeader>
              <TableColumn key="id">菜单 ID</TableColumn>
              <TableColumn key="name">菜单名称</TableColumn>
              <TableColumn key="welcome_prompt">欢迎词文件</TableColumn>
              <TableColumn key="timeout_secs">超时秒数</TableColumn>
              <TableColumn key="mappings">按键映射数</TableColumn>
              <TableColumn key="actions" align="end">操作</TableColumn>
            </TableHeader>
            <TableBody items={filteredData} emptyContent="暂无 IVR 菜单数据">
              {(record) => (
                <TableRow key={record.id}>
                  <TableCell key="id">
                    <span className="font-mono font-semibold text-foreground">{record.id}</span>
                  </TableCell>
                  <TableCell key="name">{record.name}</TableCell>
                  <TableCell key="welcome_prompt">
                    <div className="flex items-center gap-1.5 font-mono text-default-600">
                      <Volume2 className="w-3.5 h-3.5 text-default-400" />
                      <span>{record.welcome_prompt || record.welcome_file || '-'}</span>
                    </div>
                  </TableCell>
                  <TableCell key="timeout_secs">{record.timeout_secs || record.timeout_sec || 10}s</TableCell>
                  <TableCell key="mappings">
                    <Chip size="sm" variant="flat">
                      {Array.isArray(record.mappings) ? `${record.mappings.length} 规则` : '0 规则'}
                    </Chip>
                  </TableCell>
                  <TableCell key="actions">
                    <div className="flex items-center justify-end gap-1">
                      <Button isIconOnly size="sm" variant="light" onPress={() => openEdit(record)}>
                        <Pencil className="w-4 h-4 text-default-500" />
                      </Button>
                      <Button
                        isIconOnly
                        size="sm"
                        color="danger"
                        variant="light"
                        onPress={() => handleDelete(record.id)}
                      >
                        <Trash2 className="w-4 h-4 text-danger" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </CardBody>
      </Card>
      )}

      <Modal isOpen={isOpen} onClose={onClose} size="5xl">
        <ModalContent>
          {(onModalClose) => (
            <>
              <ModalHeader>{editing ? '编辑 IVR 菜单' : '新建 IVR 菜单'}</ModalHeader>
              <ModalBody className="gap-4 py-4">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <Input
                    label="菜单 ID"
                    variant="bordered"
                    placeholder="例如 main-menu"
                    value={form.id}
                    onValueChange={(v) => setForm({ ...form, id: v })}
                    isDisabled={editing}
                    isRequired
                    isInvalid={!!errors.id}
                    errorMessage={errors.id}
                  />
                  <Input
                    label="菜单名称"
                    variant="bordered"
                    placeholder="例如 主菜单"
                    value={form.name}
                    onValueChange={(v) => setForm({ ...form, name: v })}
                    isRequired
                    isInvalid={!!errors.name}
                    errorMessage={errors.name}
                  />
                  <Input
                    label="欢迎词文件"
                    variant="bordered"
                    placeholder="例如 /var/lib/vos/sounds/welcome.wav"
                    value={form.welcome_file}
                    onValueChange={(v) => setForm({ ...form, welcome_file: v })}
                  />
                  <Input
                    type="number"
                    label="超时秒数"
                    variant="bordered"
                    placeholder="1 - 60"
                    value={String(form.timeout_sec)}
                    onValueChange={(v) => setForm({ ...form, timeout_sec: Number(v) || 10 })}
                    min={1}
                    max={60}
                  />
                </div>

                <div className="flex flex-col gap-3 pt-2 border-t border-default-200">
                  <div className="flex items-center justify-between">
                    <div>
                      <h3 className="text-small font-semibold text-foreground">按键动作规则</h3>
                      <p className="text-tiny text-default-400">配置每个 DTMF 按键对应的呼叫动作</p>
                    </div>
                    <Button
                      variant="bordered"
                      size="sm"
                      onPress={addMapping}
                      startContent={<Plus className="w-4 h-4" />}
                    >
                      添加规则
                    </Button>
                  </div>

                  {form.menu_actions.length === 0 ? (
                    <div className="text-center py-8 text-default-400 text-small">
                      暂无按键规则，点击"添加规则"开始配置
                    </div>
                  ) : (
                    <div className="flex flex-col gap-2">
                      {form.menu_actions.map((mapping, idx) => (
                        <div
                          key={mapping._uid}
                          className="grid grid-cols-1 md:grid-cols-12 gap-2 p-3 bg-content2 rounded-large"
                        >
                          <div className="md:col-span-2">
                            <Select
                              aria-label="按键"
                              variant="bordered"
                              size="sm"
                              selectedKeys={mapping.key ? [mapping.key] : []}
                              onChange={(e) => updateMapping(idx, { key: e.target.value })}
                              placeholder="按键"
                            >
                              {dtmfOptions.map((k) => (
                                <SelectItem key={k}>{k}</SelectItem>
                              ))}
                            </Select>
                          </div>
                          <div className="md:col-span-3">
                            <Select
                              aria-label="动作类型"
                              variant="bordered"
                              size="sm"
                              selectedKeys={mapping.action ? [mapping.action] : []}
                              onChange={(e) => updateMapping(idx, { action: e.target.value })}
                              placeholder="动作类型"
                            >
                              {actionOptions.map((opt) => (
                                <SelectItem key={opt.value}>{opt.label}</SelectItem>
                              ))}
                            </Select>
                          </div>
                          <div className="md:col-span-3">
                            <Input
                              variant="bordered"
                              size="sm"
                              placeholder="目标 (分机/URL/TTS文本)"
                              value={mapping.target}
                              onValueChange={(v) => updateMapping(idx, { target: v })}
                            />
                          </div>
                          <div className="md:col-span-2">
                            <Input
                              variant="bordered"
                              size="sm"
                              placeholder="等待/提示音"
                              value={mapping.waiting_prompt}
                              onValueChange={(v) => updateMapping(idx, { waiting_prompt: v })}
                            />
                          </div>
                          <div className="md:col-span-2 flex items-center gap-1">
                            <Select
                              aria-label="HTTP 方法"
                              variant="bordered"
                              size="sm"
                              selectedKeys={[mapping.webhook_method || 'POST']}
                              onChange={(e) => updateMapping(idx, { webhook_method: e.target.value })}
                            >
                              <SelectItem key="POST">POST</SelectItem>
                              <SelectItem key="GET">GET</SelectItem>
                            </Select>
                            <Button
                              isIconOnly
                              size="sm"
                              color="danger"
                              variant="light"
                              onPress={() => removeMapping(idx)}
                            >
                              <Trash2 className="w-4 h-4 text-danger" />
                            </Button>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </ModalBody>
              <ModalFooter>
                <Button variant="light" onPress={onModalClose}>取消</Button>
                <Button color="primary" onPress={handleSave}>保存</Button>
              </ModalFooter>
            </>
          )}
        </ModalContent>
      </Modal>
    </div>
  );
}
