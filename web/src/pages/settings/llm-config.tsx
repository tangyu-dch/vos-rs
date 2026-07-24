// LLM 配置管理页：Table 布局 + 客户端分页 + 搜索过滤。
// 配置存储在数据库 llm_configs 表，Copilot 运行时读取 is_active=true 的记录动态调用。

import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Button, Card, CardBody, Chip, Input, Modal, ModalBody, ModalContent,
  ModalFooter, ModalHeader, Select, SelectItem, Spinner,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Pagination,
  Tooltip,
} from '@heroui/react';
import {
  Plus, RefreshCw, Trash2, CheckCircle2, Pencil, ExternalLink, Cpu, Search,
} from 'lucide-react';
import { api } from '@/services/client';
import { message } from '@/utils/toast';
import { ErrorState } from '@/components/detail-shell';
import {
  LLM_PROVIDER_PRESETS, findPreset, maskApiKey,
  type LlmConfigRecord, type UpsertLlmConfigInput,
} from './llm-presets';

const EMPTY_FORM: UpsertLlmConfigInput = {
  name: '', provider: '', api_key: '', base_url: '', model: '', temperature: 0.3,
};

const PAGE_SIZE = 10;

export function LlmConfigPage() {
  const [configs, setConfigs] = useState<LlmConfigRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState('');
  const [modalOpen, setModalOpen] = useState(false);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [form, setForm] = useState<UpsertLlmConfigInput>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);

  // 搜索 + 分页状态（客户端过滤）
  const [searchKey, setSearchKey] = useState('');
  const [page, setPage] = useState(1);

  const load = useCallback(async (silent = false) => {
    if (silent) setIsRefreshing(true);
    else setLoading(true);
    setError('');
    try {
      const list = await api.get<LlmConfigRecord[]>('/llm-configs');
      setConfigs(list);
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      setLoading(false);
      setIsRefreshing(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  // 客户端过滤：按名称/厂商/模型 模糊匹配（大小写不敏感）
  const filtered = useMemo(() => {
    const key = searchKey.trim().toLowerCase();
    if (!key) return configs;
    return configs.filter((c) =>
      c.name.toLowerCase().includes(key)
      || c.provider.toLowerCase().includes(key)
      || c.model.toLowerCase().includes(key)
      || (findPreset(c.provider)?.label ?? '').toLowerCase().includes(key)
    );
  }, [configs, searchKey]);

  // 分页计算
  const totalPages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, totalPages);
  const paged = useMemo(() => {
    const start = (currentPage - 1) * PAGE_SIZE;
    return filtered.slice(start, start + PAGE_SIZE);
  }, [filtered, currentPage]);

  // 搜索变化时重置到第 1 页
  useEffect(() => { setPage(1); }, [searchKey]);

  const openCreate = () => {
    setEditingId(null);
    setForm(EMPTY_FORM);
    setModalOpen(true);
  };

  const openEdit = (rec: LlmConfigRecord) => {
    setEditingId(rec.id);
    setForm({
      name: rec.name, provider: rec.provider, api_key: rec.api_key,
      base_url: rec.base_url, model: rec.model, temperature: rec.temperature,
    });
    setModalOpen(true);
  };

  const onPresetChange = (provider: string) => {
    const preset = findPreset(provider);
    setForm((f) => ({
      ...f,
      provider,
      // 仅在切换到不同厂商时清除 API Key（不同厂商 key 不通用）
      api_key: provider === f.provider ? f.api_key : '',
      base_url: preset?.baseUrl ?? f.base_url,
      model: preset?.models[0] ?? f.model,
    }));
  };

  const handleSave = async () => {
    if (!form.name.trim() || !form.provider.trim() || !form.api_key.trim() || !form.base_url.trim() || !form.model.trim()) {
      message.warning('请选择厂商并填写名称、API Key、Base URL 和模型');
      return;
    }
    setSaving(true);
    try {
      if (editingId !== null) {
        await api.put<LlmConfigRecord>(`/llm-configs/${editingId}`, form);
        message.success('配置已更新');
      } else {
        await api.post<LlmConfigRecord>('/llm-configs', form);
        message.success('配置已创建');
      }
      setModalOpen(false);
      await load(true);
    } catch (e) {
      message.error(e instanceof Error ? e.message : '保存失败');
    } finally {
      setSaving(false);
    }
  };

  const handleActivate = async (id: number) => {
    try {
      await api.post<LlmConfigRecord>(`/llm-configs/${id}/activate`);
      message.success('已切换为当前启用模型');
      await load(true);
    } catch (e) {
      message.error(e instanceof Error ? e.message : '启用失败');
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await api.delete(`/llm-configs/${id}`);
      message.success('配置已删除');
      setConfirmDeleteId(null);
      await load(true);
    } catch (e) {
      message.error(e instanceof Error ? e.message : '删除失败');
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Spinner size="lg" />
      </div>
    );
  }
  if (error) {
    return <ErrorState error={error} retry={load} />;
  }

  const activePreset = findPreset(form.provider);

  return (
    <div className="flex flex-col gap-5 w-full">
      {/* 页头 */}
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="min-w-0">
          <h1 className="text-base font-bold flex items-center gap-2">
            <Cpu className="w-5 h-5 text-primary" />
            LLM 配置管理
          </h1>
          <p className="text-sm text-default-500 mt-1">
            管理大模型厂商配置，Copilot 运行时使用当前启用的模型进行智能分析。切换模型无需重启服务。
          </p>
        </div>
        <div className="flex gap-2 shrink-0">
          <Button variant="flat" size="sm" startContent={<RefreshCw className="w-3.5 h-3.5" />} onClick={() => void load(true)}>
            刷新
          </Button>
          <Button color="primary" size="sm" startContent={<Plus className="w-3.5 h-3.5" />} onClick={openCreate}>
            新建配置
          </Button>
        </div>
      </div>

      {/* Table 卡片 */}
      <Card shadow="sm" className="w-full">
        <CardBody className="p-4 flex flex-col gap-3">
          {/* 工具栏：搜索 + 计数 */}
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <Input
              size="sm"
              isClearable
              variant="bordered"
              className="max-w-xs"
              placeholder="搜索名称 / 厂商 / 模型"
              value={searchKey}
              onValueChange={setSearchKey}
              startContent={<Search className="w-3.5 h-3.5 text-default-400" />}
            />
            <span className="text-tiny text-default-400">
              共 {filtered.length} 条{searchKey ? `（已过滤，原 ${configs.length} 条）` : ''}
            </span>
          </div>

          {/* 配置表格 */}
          <Table
            aria-label="LLM 配置列表"
            isStriped
            removeWrapper
            bottomContent={
              totalPages > 1 ? (
                <div className="flex items-center justify-between pt-3 border-t border-divider">
                  <span className="text-tiny text-default-400">
                    第 {currentPage} / {totalPages} 页 · 每页 {PAGE_SIZE} 条
                  </span>
                  <Pagination
                    total={totalPages}
                    page={currentPage}
                    color="primary"
                    size="sm"
                    onChange={setPage}
                  />
                </div>
              ) : undefined
            }
          >
            <TableHeader>
              <TableColumn key="status" width={90}>状态</TableColumn>
              <TableColumn key="name">名称</TableColumn>
              <TableColumn key="provider" width={140}>厂商</TableColumn>
              <TableColumn key="model">模型</TableColumn>
              <TableColumn key="base_url">Base URL</TableColumn>
              <TableColumn key="api_key" width={160}>API Key</TableColumn>
              <TableColumn key="temperature" width={70} align="center">温度</TableColumn>
              <TableColumn key="updated_at" width={160}>更新时间</TableColumn>
              <TableColumn key="actions" width={200} align="end">操作</TableColumn>
            </TableHeader>
            <TableBody
              items={paged}
              className={isRefreshing ? 'opacity-50 transition-opacity duration-300' : 'transition-opacity duration-300'}
              emptyContent={searchKey ? '没有匹配的配置' : '暂无 LLM 配置，点击「新建配置」添加第一个大模型厂商'}
            >
              {(cfg) => {
                const preset = findPreset(cfg.provider);
                return (
                  <TableRow key={cfg.id}>
                    <TableCell key="status">
                      {cfg.is_active ? (
                        <Chip size="sm" color="primary" variant="flat" startContent={<CheckCircle2 className="w-3 h-3" />}>
                          启用中
                        </Chip>
                      ) : (
                        <span className="text-tiny text-default-400">—</span>
                      )}
                    </TableCell>
                    <TableCell key="name">
                      <span className="font-semibold text-foreground">{cfg.name}</span>
                    </TableCell>
                    <TableCell key="provider">
                      <Chip size="sm" variant="flat" className="text-default-500">
                        {preset?.label ?? cfg.provider}
                      </Chip>
                    </TableCell>
                    <TableCell key="model">
                      <span className="font-mono text-xs text-foreground">{cfg.model}</span>
                    </TableCell>
                    <TableCell key="base_url">
                      <span className="font-mono text-xs text-default-600 break-all">{cfg.base_url}</span>
                    </TableCell>
                    <TableCell key="api_key">
                      <span className="font-mono text-xs text-default-600">{maskApiKey(cfg.api_key)}</span>
                    </TableCell>
                    <TableCell key="temperature" align="center">
                      <span className="font-mono text-xs">{cfg.temperature}</span>
                    </TableCell>
                    <TableCell key="updated_at">
                      <span className="text-xs text-default-500">
                        {new Date(cfg.updated_at).toLocaleString('zh-CN', { hour12: false })}
                      </span>
                    </TableCell>
                    <TableCell key="actions">
                      <div className="flex items-center justify-end gap-1">
                        {!cfg.is_active && (
                          <Button size="sm" color="primary" variant="flat" onClick={() => handleActivate(cfg.id)}>
                            启用
                          </Button>
                        )}
                        <Tooltip content="编辑" placement="top">
                          <Button
                            isIconOnly
                            size="sm"
                            variant="light"
                            aria-label="编辑"
                            onClick={() => openEdit(cfg)}
                          >
                            <Pencil className="w-3.5 h-3.5" />
                          </Button>
                        </Tooltip>
                        <Tooltip content="删除" placement="top" color="danger">
                          <Button
                            isIconOnly
                            size="sm"
                            variant="light"
                            color="danger"
                            aria-label="删除"
                            onClick={() => setConfirmDeleteId(cfg.id)}
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </Button>
                        </Tooltip>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              }}
            </TableBody>
          </Table>
        </CardBody>
      </Card>

      {/* 新建/编辑 Modal */}
      <Modal isOpen={modalOpen} onClose={() => setModalOpen(false)} size="2xl">
        <ModalContent>
          <ModalHeader>{editingId !== null ? '编辑 LLM 配置' : '新建 LLM 配置'}</ModalHeader>
          <ModalBody className="gap-4">
            {/* 厂商预设选择 */}
            <div>
              <label className="text-sm font-medium text-foreground mb-1.5 block">厂商预设</label>
              <div className="flex flex-wrap gap-2">
                {LLM_PROVIDER_PRESETS.map((p) => (
                  <Button
                    key={p.provider}
                    size="sm"
                    variant={form.provider === p.provider ? 'solid' : 'flat'}
                    color={form.provider === p.provider ? 'primary' : 'default'}
                    onClick={() => onPresetChange(p.provider)}
                  >
                    {p.label}
                  </Button>
                ))}
              </div>
              {activePreset?.apiKeyUrl && (
                <a href={activePreset.apiKeyUrl} target="_blank" rel="noopener noreferrer"
                   className="inline-flex items-center gap-1 text-xs text-primary mt-2 hover:underline">
                  <ExternalLink className="w-3 h-3" /> 前往 {activePreset.label} 申请 API Key
                </a>
              )}
            </div>

            <Input
              label="配置名称" labelPlacement="outside" placeholder="如：智谱生产环境"
              value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
            <Input
              label="API Key" labelPlacement="outside" placeholder="sk-..."
              type="password"
              value={form.api_key} onChange={(e) => setForm({ ...form, api_key: e.target.value })}
            />
            <Input
              label="Base URL" labelPlacement="outside" placeholder="https://..."
              value={form.base_url} onChange={(e) => setForm({ ...form, base_url: e.target.value })}
              description="OpenAI 兼容的 chat completions 基础地址"
            />
            <div className="flex gap-3">
              {activePreset && activePreset.models.length > 0 ? (
                <Select
                  className="flex-1"
                  label="模型" labelPlacement="outside"
                  selectedKeys={[form.model]}
                  onChange={(e) => setForm({ ...form, model: e.target.value })}
                >
                  {activePreset.models.map((m) => <SelectItem key={m}>{m}</SelectItem>)}
                </Select>
              ) : (
                <Input
                  className="flex-1"
                  label="模型" labelPlacement="outside" placeholder="glm-4.7-flash"
                  value={form.model} onChange={(e) => setForm({ ...form, model: e.target.value })}
                />
              )}
              <Input
                className="w-32"
                label="温度" labelPlacement="outside" type="number" step="0.1" min="0" max="2"
                value={String(form.temperature)}
                onChange={(e) => setForm({ ...form, temperature: parseFloat(e.target.value) || 0 })}
              />
            </div>
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onClick={() => setModalOpen(false)}>取消</Button>
            <Button color="primary" isLoading={saving} onClick={handleSave}>
              {editingId !== null ? '保存' : '创建'}
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>

      {/* 删除确认 */}
      <Modal isOpen={confirmDeleteId !== null} onClose={() => setConfirmDeleteId(null)} size="sm">
        <ModalContent>
          <ModalHeader>确认删除</ModalHeader>
          <ModalBody>
            确定要删除这条 LLM 配置吗？此操作不可撤销。
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onClick={() => setConfirmDeleteId(null)}>取消</Button>
            <Button color="danger" onClick={() => confirmDeleteId && handleDelete(confirmDeleteId)}>
              删除
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </div>
  );
}
