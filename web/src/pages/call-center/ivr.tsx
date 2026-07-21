import { useCallback, useEffect, useState } from 'react';
import {
  Card, CardBody, Button, Chip, Input, Table, TableHeader, TableColumn,
  TableBody, TableRow, TableCell, Switch, Modal, ModalBody, ModalHeader,
  ModalContent, ModalFooter, useDisclosure,
} from '@heroui/react';
import { Plus, RefreshCw, Pencil, Trash2, Search, GitFork, Network } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState, LoadingState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import { IvrTopologyEditor, type IvrFlowFields } from '@/components/ivr/ivr-rule-binding';

interface IvrListItem {
  id: string;
  name: string;
  description?: string;
  did?: string;
  welcome_prompt?: string;
  timeout_secs?: number;
  enabled?: boolean;
  node_count?: number;
  created_at?: string;
  updated_at?: string;
}

const emptyForm = {
  id: '',
  name: '',
  description: '',
  did: '',
  welcome_prompt: 'welcome.wav',
  timeout_secs: 30,
  enabled: true,
};

export default function IvrPage() {
  const [data, setData] = useState<IvrListItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [searchKey, setSearchKey] = useState('');
  const [form, setForm] = useState(emptyForm);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState(false);
  const { isOpen, onOpen, onClose } = useDisclosure();
  // 当前正在编辑拓扑的 IVR (null 表示未打开拓扑编排 Modal)
  const [topoIvr, setTopoIvr] = useState<IvrFlowFields | null>(null);

  const loadData = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const res: any = await api.get('/ivr/menus');
      const items = res.items || res.data || [];
      setData(items.map((item: any) => ({
        id: item.id,
        name: item.name,
        description: item.description,
        did: item.did,
        welcome_prompt: item.welcome_prompt,
        timeout_secs: item.timeout_secs,
        enabled: item.enabled ?? true,
        node_count: Array.isArray(item.mappings) ? item.mappings.length : (item.node_count ?? 0),
        created_at: item.created_at,
        updated_at: item.updated_at,
      })));
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载 IVR 列表失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void loadData(); }, [loadData]);

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
      description: form.description,
      did: form.did,
      welcome_prompt: form.welcome_prompt,
      timeout_secs: form.timeout_secs,
      enabled: form.enabled,
      nodes: [],
      edges: [],
    };
    try {
      if (editing) {
        await api.put(`/ivr/menus/${form.id}`, payload);
        message.success('IVR 基础信息已更新');
      } else {
        await api.post('/ivr/menus', payload);
        message.success('IVR 已创建，点击列表中的「拓扑编排」按钮开始配置节点');
      }
      onClose();
      void loadData();
    } catch (err) {
      message.error(err instanceof Error ? err.message : '保存 IVR 失败');
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm(`确定要删除 IVR 流程 ${id} 吗？此操作不可撤销。`)) return;
    try {
      await api.delete(`/ivr/menus/${id}`);
      message.success('IVR 已删除');
      void loadData();
    } catch (err) {
      message.error(err instanceof Error ? err.message : '删除失败');
    }
  };

  const handleToggleEnabled = async (item: IvrListItem, enabled: boolean) => {
    try {
      await api.put(`/ivr/menus/${item.id}`, { ...item, enabled });
      message.success(enabled ? '已启用' : '已停用');
      void loadData();
    } catch (err) {
      message.error(err instanceof Error ? err.message : '切换状态失败');
    }
  };

  const openCreate = () => {
    setEditing(false);
    setForm({ ...emptyForm, id: `ivr-${Date.now().toString().slice(-6)}` });
    setErrors({});
    onOpen();
  };

  const openEdit = (item: IvrListItem) => {
    setEditing(true);
    setForm({
      id: item.id,
      name: item.name,
      description: item.description ?? '',
      did: item.did ?? '',
      welcome_prompt: item.welcome_prompt ?? 'welcome.wav',
      timeout_secs: item.timeout_secs ?? 30,
      enabled: item.enabled ?? true,
    });
    setErrors({});
    onOpen();
  };

  // 打开拓扑编排 Modal (与 routes 页一致: 表格行 → 独立画布 Modal)
  const openCanvas = (item: IvrListItem) => {
    setTopoIvr({
      id: item.id,
      name: item.name,
      description: item.description,
      did: item.did,
      welcome_prompt: item.welcome_prompt,
      timeout_secs: item.timeout_secs,
      enabled: item.enabled,
    });
  };

  // 拓扑保存成功后刷新表格 (节点数等可能变化)
  const handleTopologySaved = () => {
    void loadData();
  };

  const filteredData = data.filter(
    (item) =>
      item.id.toLowerCase().includes(searchKey.toLowerCase()) ||
      item.name.toLowerCase().includes(searchKey.toLowerCase()) ||
      (item.did ?? '').toLowerCase().includes(searchKey.toLowerCase())
  );

  return (
    <div className="flex flex-col gap-4">
      {/* 顶部标题栏 */}
      <div className="flex flex-wrap items-center justify-between gap-4 p-5 bg-content1 rounded-2xl border border-default-200 dark:border-slate-800">
        <div className="flex items-center gap-3.5">
          <div className="w-11 h-11 rounded-2xl bg-purple-500/15 flex items-center justify-center text-purple-600">
            <GitFork className="w-6 h-6" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-bold">IVR 多级语音导航</h2>
              <Chip size="sm" color="secondary" variant="flat">拖拽编排</Chip>
            </div>
            <p className="text-xs text-default-500 mt-0.5">
              支持多级多节点拖拽编排，18 种节点类型覆盖播放/收号/分支/转接/AI 对话等场景
            </p>
          </div>
        </div>
        <Button
          color="secondary"
          className="font-bold text-white"
          startContent={<Plus className="w-4 h-4" />}
          onPress={openCreate}
        >
          定义新 IVR
        </Button>
      </div>

      {/* IVR 列表 Table */}
      <Card className="shadow-sm">
        <CardBody className="p-5 flex flex-col gap-4">
          <div className="flex items-center justify-between gap-4">
            <Input
              className="max-w-xs"
              placeholder="搜索 ID / 名称 / DID..."
              startContent={<Search className="w-4 h-4 text-default-400" />}
              value={searchKey}
              onValueChange={setSearchKey}
              isClearable
              onClear={() => setSearchKey('')}
            />
            <Button size="sm" variant="flat" startContent={<RefreshCw className="w-3.5 h-3.5" />} onPress={loadData}>
              刷新
            </Button>
          </div>

          {error ? (
            <ErrorState error={error} retry={loadData} />
          ) : loading ? (
            <LoadingState />
          ) : (
            <Table aria-label="IVR 流程列表">
              <TableHeader>
                <TableColumn>IVR ID</TableColumn>
                <TableColumn>名称</TableColumn>
                <TableColumn>绑定 DID</TableColumn>
                <TableColumn>欢迎语音</TableColumn>
                <TableColumn>节点数</TableColumn>
                <TableColumn>超时</TableColumn>
                <TableColumn align="center">启用</TableColumn>
                <TableColumn align="end">操作</TableColumn>
              </TableHeader>
              <TableBody emptyContent="暂无 IVR 流程，点击右上角定义新 IVR 开始">
                {filteredData.map((item) => (
                  <TableRow key={item.id}>
                    <TableCell className="font-mono font-bold text-purple-600">{item.id}</TableCell>
                    <TableCell className="font-semibold">{item.name}</TableCell>
                    <TableCell>
                      {item.did ? (
                        <Chip size="sm" variant="flat" color="primary">{item.did}</Chip>
                      ) : (
                        <span className="text-default-400 text-xs">未绑定</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <Chip size="sm" variant="flat">{item.welcome_prompt ?? 'welcome.wav'}</Chip>
                    </TableCell>
                    <TableCell>
                      <Chip size="sm" color="secondary" variant="dot">
                        {item.node_count ?? 0} 节点
                      </Chip>
                    </TableCell>
                    <TableCell>{item.timeout_secs ?? 30} 秒</TableCell>
                    <TableCell>
                      <div className="flex justify-center">
                        <Switch
                          size="sm"
                          isSelected={item.enabled ?? true}
                          onChange={(e) => handleToggleEnabled(item, e.target.checked)}
                        />
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center justify-end gap-1.5">
                        <Button
                          size="sm"
                          color="secondary"
                          variant="flat"
                          className="font-bold"
                          startContent={<Network className="w-3.5 h-3.5" />}
                          onPress={() => openCanvas(item)}
                        >
                          拓扑编排
                        </Button>
                        <Button
                          isIconOnly
                          size="sm"
                          variant="light"
                          onPress={() => openEdit(item)}
                        >
                          <Pencil className="w-4 h-4" />
                        </Button>
                        <Button
                          isIconOnly
                          size="sm"
                          color="danger"
                          variant="light"
                          onPress={() => handleDelete(item.id)}
                        >
                          <Trash2 className="w-4 h-4" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardBody>
      </Card>

      {/* 创建/编辑基础信息 Modal */}
      <Modal isOpen={isOpen} onOpenChange={(o) => !o && onClose()} size="2xl" scrollBehavior="inside">
        <ModalContent>
          {(onModalClose) => (
            <>
              <ModalHeader className="flex flex-col gap-1 border-b border-default-200 dark:border-slate-800">
                <div className="flex items-center gap-2">
                  <GitFork className="w-5 h-5 text-purple-600" />
                  <span>{editing ? '编辑 IVR 基础信息' : '定义新 IVR 流程'}</span>
                </div>
                <p className="text-xs font-normal text-default-500">
                  先填写基础信息创建 IVR，保存后在列表中点击「拓扑编排」按钮即可拖拽配置多级节点
                </p>
              </ModalHeader>
              <ModalBody className="p-6">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <Input
                    label="IVR ID"
                    placeholder="例如 ivr-main-sales"
                    value={form.id}
                    onValueChange={(v) => setForm({ ...form, id: v })}
                    isInvalid={!!errors.id}
                    errorMessage={errors.id}
                    isDisabled={editing}
                  />
                  <Input
                    label="流程名称"
                    placeholder="例如 售前客服多级导航"
                    value={form.name}
                    onValueChange={(v) => setForm({ ...form, name: v })}
                    isInvalid={!!errors.name}
                    errorMessage={errors.name}
                  />
                  <Input
                    label="绑定 DID 号码"
                    placeholder="例如 4008009000"
                    value={form.did}
                    onValueChange={(v) => setForm({ ...form, did: v })}
                  />
                  <Input
                    label="欢迎语音文件"
                    placeholder="welcome.wav"
                    value={form.welcome_prompt}
                    onValueChange={(v) => setForm({ ...form, welcome_prompt: v })}
                  />
                  <Input
                    type="number"
                    label="全局超时 (秒)"
                    value={String(form.timeout_secs)}
                    onValueChange={(v) => setForm({ ...form, timeout_secs: Number(v) || 30 })}
                    min={1}
                    max={600}
                  />
                  <div className="flex flex-col gap-1.5">
                    <label className="text-sm font-semibold">启用状态</label>
                    <Switch
                      isSelected={form.enabled}
                      onChange={(e) => setForm({ ...form, enabled: e.target.checked })}
                    />
                  </div>
                  <div className="md:col-span-2">
                    <Input
                      label="流程描述"
                      placeholder="简要描述此 IVR 流程的用途"
                      value={form.description}
                      onValueChange={(v) => setForm({ ...form, description: v })}
                    />
                  </div>
                </div>
              </ModalBody>
              <ModalFooter>
                <Button variant="flat" onPress={onModalClose}>取消</Button>
                <Button color="secondary" className="font-bold text-white" onPress={handleSave}>
                  {editing ? '保存基础信息' : '创建 IVR'}
                </Button>
              </ModalFooter>
            </>
          )}
        </ModalContent>
      </Modal>

      {/* IVR 拓扑编排 Modal (每个 IVR 独立画布, 与 routes 页一致) */}
      <Modal
        isOpen={topoIvr !== null}
        onOpenChange={(o) => !o && setTopoIvr(null)}
        size="full"
        scrollBehavior="inside"
        classNames={{
          base: 'h-screen max-h-screen w-screen max-w-screen',
          wrapper: 'h-screen max-h-screen',
        }}
      >
        <ModalContent className="h-full">
          <ModalHeader className="flex items-center gap-2 border-b border-default-200 dark:border-slate-800 shrink-0">
            <Network className="w-5 h-5 text-purple-600" />
            <span>IVR 拓扑编排</span>
            {topoIvr && (
              <>
                <Chip size="sm" variant="flat" color="secondary" className="ml-2">
                  {topoIvr.id}
                </Chip>
                {topoIvr.did && (
                  <Chip size="sm" variant="flat" color="primary">DID {topoIvr.did}</Chip>
                )}
              </>
            )}
          </ModalHeader>
          <ModalBody className="flex-1 min-h-0 p-4 overflow-hidden flex flex-col">
            {topoIvr && (
              <div className="flex-1 min-h-0">
                <IvrTopologyEditor flow={topoIvr} onSaved={handleTopologySaved} />
              </div>
            )}
          </ModalBody>
        </ModalContent>
      </Modal>
    </div>
  );
}
