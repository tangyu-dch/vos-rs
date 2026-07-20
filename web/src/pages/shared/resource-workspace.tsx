// 资源工作台：通用 CRUD 列表 + 表单 + 分页 + 搜索 + 状态筛选
// 从 console.tsx 拆分

import { useCallback, useEffect, useState } from 'react';
import {
  Button, Card, CardBody, Input, Select, SelectItem, Pagination,
  Chip, Switch, Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Textarea,
} from '@heroui/react';
import { Plus, RefreshCw, Search, Eye, Pencil, Trash2 } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { api } from '@/services/client';
import {
  createResource, deleteResource, listResource, updateResource, type Entity,
} from '@/services/resources';
import { listOptions, trunkRole } from '@/services/trunks';
import { ErrorState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import {
  callDetailText, entityId, valueText, moneyText, durationSecondsText,
} from '@/pages/shared/format';
import type { FieldSpec, ResourceSpec, SelectOptionSpec } from '@/pages/shared/types';

const moneyFields = new Set([
  'balance', 'credit_limit', 'price_per_interval', 'amount', 'balance_after', 'cost',
]);

export function usePageVisibility() {
  const [isVisible, setIsVisible] = useState(!document.hidden);
  useEffect(() => {
    const handleVisibilityChange = () => setIsVisible(!document.hidden);
    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => document.removeEventListener('visibilitychange', handleVisibilityChange);
  }, []);
  return isVisible;
}

export function ConfirmDialog({
  open, title, message, loading, onConfirm, onClose,
}: {
  open: boolean;
  title: string;
  message: string;
  loading?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Modal isOpen={open} onOpenChange={(o) => !o && onClose()} size="sm">
      <ModalContent>
        <ModalHeader>{title}</ModalHeader>
        <ModalBody>
          <p className="text-small text-default-500">{message}</p>
        </ModalBody>
        <ModalFooter>
          <Button variant="flat" onPress={onClose}>取消</Button>
          <Button color="danger" isLoading={loading} onPress={onConfirm}>确认</Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}

export function FormControl({
  field, disabled = false, value, onChange,
}: {
  field: FieldSpec;
  disabled?: boolean;
  value?: unknown;
  onChange: (value: unknown) => void;
}) {
  if (field.kind === 'number') {
    return (
      <Input
        type="number"
        variant="bordered"
        isDisabled={disabled}
        min={field.min ?? 0}
        placeholder={field.placeholder}
        value={value !== undefined && value !== null ? String(value) : ''}
        onValueChange={(v) => onChange(v === '' ? undefined : Number(v))}
      />
    );
  }
  if (field.kind === 'switch') {
    return (
      <Switch isDisabled={disabled} isSelected={Boolean(value)} onValueChange={(v) => onChange(v)} />
    );
  }
  if (field.kind === 'select') {
    const options = (field.options || []).map((option) => typeof option === 'string' ? { label: option, value: option } : option);
    const selected = value !== undefined && value !== null ? [String(value)] : [];
    return (
      <Select
        variant="bordered"
        isDisabled={disabled}
        placeholder={field.placeholder}
        selectedKeys={selected}
        onChange={(e) => onChange(e.target.value)}
      >
        {options.map((option) => (
          <SelectItem key={option.value}>{option.label}</SelectItem>
        ))}
      </Select>
    );
  }
  if (field.kind === 'secret') {
    return (
      <Input
        type="password"
        variant="bordered"
        isDisabled={disabled}
        placeholder={field.placeholder}
        value={String(value ?? '')}
        onValueChange={(v) => onChange(v)}
      />
    );
  }
  if (field.kind === 'textarea') {
    return (
      <Textarea
        variant="bordered"
        isDisabled={disabled}
        placeholder={field.placeholder}
        minRows={3}
        maxRows={7}
        value={String(value ?? '')}
        onValueChange={(v) => onChange(v)}
      />
    );
  }
  return (
    <Input
      variant="bordered"
      isDisabled={disabled}
      placeholder={field.placeholder}
      value={String(value ?? '')}
      onValueChange={(v) => onChange(v)}
    />
  );
}

export function resourceFormValues(spec: ResourceSpec, row: Entity | null): Entity {
  if (row) {
    if (spec.path === '/numbers') {
      const direction = String(row.direction ?? 'both');
      return {
        ...row,
        can_receive: row.can_receive ?? ['inbound', 'both', 'bidirectional'].includes(direction),
        can_present: row.can_present ?? ['outbound', 'both', 'bidirectional'].includes(direction),
      };
    }
    return { ...row };
  }
  return spec.fields.reduce<Entity>((defaults, field) => {
    if (field.defaultValue !== undefined) defaults[field.key] = field.defaultValue;
    else if (field.kind === 'switch') defaults[field.key] = false;
    if (defaults[field.key] === undefined && field.required && field.kind === 'select' && field.options?.[0]) {
      const option = field.options[0];
      defaults[field.key] = typeof option === 'string' ? option : option.value;
    }
    return defaults;
  }, {});
}

export function resourceSaveValues(spec: ResourceSpec, values: Entity, editing: boolean): Entity {
  const result = { ...values };
  if (spec.path === '/numbers') {
    result.direction = result.can_receive ? (result.can_present ? 'both' : 'inbound') : (result.can_present ? 'outbound' : 'disabled');
  }
  if (!editing) return result;
  spec.fields.filter((field) => field.kind === 'secret' && field.preserveEmptyOnEdit).forEach((field) => {
    if (result[field.key] === '' || result[field.key] === undefined) delete result[field.key];
  });
  return result;
}

export function FieldLabel({ label, required }: { label: string; required?: boolean }) {
  return (
    <label className="block text-tiny font-medium text-foreground mb-1.5">
      {label}
      {required && <span className="text-danger ml-0.5">*</span>}
    </label>
  );
}

export function ResourceWorkspace({ spec }: { spec: ResourceSpec }) {
  const [rows, setRows] = useState<Entity[]>([]);
  const [pagination, setPagination] = useState({ page: 1, page_size: 20, total: 0, total_pages: 0 });
  const [query, setQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState<'all' | 'enabled' | 'disabled'>('all');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState<Entity | null | undefined>(undefined);
  const [draft, setDraft] = useState<Entity>({});
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const [actionRow, setActionRow] = useState<Entity | null>(null);
  const [amount, setAmount] = useState<number>(100);
  const [fieldOptions, setFieldOptions] = useState<Record<string, SelectOptionSpec[]>>({});
  const [confirmRow, setConfirmRow] = useState<Entity | null>(null);
  const navigate = useNavigate();

  const load = useCallback(async (page = pagination.page) => {
    setLoading(true); setError('');
    try {
      const result = await listResource(spec.path, { page, page_size: pagination.page_size, ...spec.params });
      setRows(result.items || []);
      setPagination(result.pagination || { page, page_size: 20, total: result.items?.length || 0, total_pages: 1 });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, [pagination.page, pagination.page_size, spec.path, spec.params]);

  useEffect(() => { void load(1); }, [spec.path]);
  useEffect(() => {
    const needsEgress = spec.fields.some((field) => field.optionsResource === 'egress-trunks');
    const needsSource = spec.fields.some((field) => field.optionsResource === 'allocation-source');
    if (!needsEgress && !needsSource) return;
    void Promise.all([listOptions('/trunks'), needsSource ? listOptions('/extensions') : Promise.resolve([])]).then(([trunks, extensions]) => setFieldOptions({
      owner_egress_trunk_id: trunks.filter((item) => trunkRole(item) === 'egress').map((item) => ({ label: String(item.id), value: String(item.id) })),
      allocation_trunks: trunks.filter((item) => trunkRole(item) === 'access').map((item) => ({ label: String(item.id), value: String(item.id) })),
      allocation_extensions: extensions.map((item) => ({ label: String(item.display_name ?? item.username), value: String(item.username) })),
    })).catch(() => setFieldOptions({ owner_egress_trunk_id: [], allocation_trunks: [], allocation_extensions: [] }));
  }, [spec.path]);

  const optionsForField = (field: FieldSpec) => {
    if (field.optionsResource === 'allocation-source') {
      const sourceType = field.key === 'owner_source_id' ? draft.owner_source_type : draft.allocation_source_type;
      return sourceType === 'trunk' ? fieldOptions.allocation_trunks || [] : fieldOptions.allocation_extensions || [];
    }
    return field.optionsResource ? fieldOptions[field.key] || [] : field.options;
  };

  const isEditing = editing !== undefined && editing !== null;
  const openForm = async (row: Entity | null) => {
    let values = resourceFormValues(spec, row);
    if (row && spec.path === '/numbers') {
      try {
        const allocations = await api.get<Entity[]>(`/numbers/${encodeURIComponent(entityId(row, spec.idKey))}/allocations`);
        const active = allocations.find((allocation) => allocation.enabled !== false);
        if (active) values = { ...values, allocation_source_type: active.source_type, allocation_source_id: active.source_id };
      } catch (reason) {
        message.warning(reason instanceof Error ? reason.message : '号码授权加载失败');
      }
    }
    setDraft(values);
    setValidationErrors({});
    setEditing(row);
  };
  const updateDraft = (key: string, value: unknown) => {
    setDraft((current) => ({
      ...current,
      [key]: value,
      ...(key === 'allocation_source_type' ? { allocation_source_id: '' } : {}),
      ...(key === 'owner_source_type' ? { owner_source_id: '' } : {}),
    }));
    setValidationErrors((current) => {
      if (!current[key]) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  };
  const save = async () => {
    try {
      const visibleFields = spec.fields.filter((field) => !field.showWhen || field.showWhen(draft));
      const errors = visibleFields.reduce<Record<string, string>>((result, field) => {
        if (field.readonly || (isEditing && field.preserveEmptyOnEdit)) return result;
        const value = draft[field.key];
        const isEmpty = value === undefined || value === null || value === '';
        if (field.required && isEmpty) result[field.key] = `请填写${field.label}`;
        else if (!isEmpty && field.pattern && !field.pattern.test(String(value))) result[field.key] = field.patternMessage || `${field.label}格式不正确`;
        else if (!isEmpty && field.min !== undefined && Number(value) < field.min) result[field.key] = `${field.label}不能小于 ${field.min}`;
        return result;
      }, {});
      if (Object.keys(errors).length) { setValidationErrors(errors); return; }
      const values = { ...resourceSaveValues(spec, draft, isEditing), ...spec.params };
      const allocation = spec.path === '/numbers' ? {
        source_type: String(values.allocation_source_type ?? ''),
        source_id: String(values.allocation_source_id ?? ''),
        enabled: true,
      } : null;
      delete values.allocation_source_type;
      delete values.allocation_source_id;
      setSaving(true);
      if (isEditing) await updateResource(spec.path, entityId(editing as Entity, spec.idKey), values);
      else await createResource(spec.path, values);
      if (allocation) {
        const number = isEditing ? entityId(editing as Entity, spec.idKey) : String(values.number);
        await api.put(`/numbers/${encodeURIComponent(number)}/allocations`, { items: [allocation] });
      }
      message.success(isEditing ? '已保存更改' : '已创建');
      setEditing(undefined);
      await load();
    } catch (reason) {
      if (reason instanceof Error) message.error(reason.message);
    } finally {
      setSaving(false);
    }
  };
  const remove = async (row: Entity) => {
    try {
      await deleteResource(spec.path, entityId(row, spec.idKey));
      message.success('已删除');
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '删除失败');
    }
  };
  const runAction = async () => {
    if (!actionRow || spec.action !== 'credit') return;
    try {
      setSaving(true);
      await api.post(`${spec.path}/${encodeURIComponent(entityId(actionRow, spec.idKey))}/credit`, { amount });
      message.success('充值成功');
      setActionRow(null);
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '操作失败');
    } finally {
      setSaving(false);
    }
  };

  const normalizedQuery = query.trim().toLowerCase();
  let visibleRows = normalizedQuery
    ? rows.filter((row) => Object.values(row).some((value) => String(value ?? '').toLowerCase().includes(normalizedQuery)))
    : rows;
  if (statusFilter !== 'all') {
    const target = statusFilter === 'enabled';
    visibleRows = visibleRows.filter((row) => (row.enabled === target || (row.enabled === undefined && target)));
  }

  const visibleFields = spec.fields.filter((field) => field.kind !== 'secret').slice(0, 7);

  const renderCell = (row: Entity, field: FieldSpec) => {
    const value = row[field.key];
    const callText = spec.path === '/calls' ? callDetailText(value, field.key) : undefined;
    if (['status', 'state', 'enabled', 'health'].includes(field.key)) {
      const positive = ['active', 'online', 'registered', 'healthy', 'answered', 'enabled', 'closed', true].includes(value as never);
      return (
        <Chip size="sm" color={positive ? 'success' : 'danger'} variant="flat">
          {callText ?? (typeof value === 'boolean' ? (value ? '启用' : '停用') : valueText(value))}
        </Chip>
      );
    }
    let text: string;
    if (callText) text = callText;
    else if (field.kind === 'duration') text = durationSecondsText(value);
    else if (moneyFields.has(field.key)) text = moneyText(value);
    else if (field.kind === 'select') {
      const options = (field.options || fieldOptions[field.key] || []).map((option) => typeof option === 'string' ? { label: option, value: option } : option);
      const actual = field.key === 'role' ? trunkRole(row) : value;
      text = valueText(options.find((option) => option.value === String(actual))?.label ?? (field.key === 'role' ? (trunkRole(row) === 'access' ? '接入中继' : '落地中继') : value));
    } else text = valueText(value);
    return <span className={field.key.includes('id') || field.key.includes('number') ? 'font-mono text-foreground' : 'text-default-600'}>{text}</span>;
  };

  return (
    <>
      <Card shadow="sm" className="p-2">
        <CardBody className="p-4 flex flex-col gap-4">
          <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-divider">
            <div>
              <h2 className="text-base font-bold text-foreground">{spec.title}</h2>
              {spec.description && <p className="text-tiny text-default-500 mt-0.5">{spec.description}</p>}
            </div>
            <div className="flex items-center gap-2">
              <Button variant="flat" size="sm" isLoading={loading} onPress={() => load()} startContent={<RefreshCw className="w-4 h-4" />}>
                刷新
              </Button>
              {!spec.readOnly && (
                <Button color="primary" size="sm" onPress={() => void openForm(null)} startContent={<Plus className="w-4 h-4" />}>
                  {spec.createLabel || '新建'}
                </Button>
              )}
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex items-center gap-3">
              <Input
                placeholder="搜索当前列表..."
                variant="bordered"
                size="sm"
                className="w-56"
                startContent={<Search className="w-4 h-4 text-default-400" />}
                value={query}
                onValueChange={setQuery}
                isClearable
                onClear={() => setQuery('')}
              />
              <Select
                aria-label="状态筛选"
                variant="bordered"
                size="sm"
                className="w-36"
                selectedKeys={[statusFilter]}
                onChange={(e) => setStatusFilter(e.target.value as 'all' | 'enabled' | 'disabled')}
              >
                <SelectItem key="all">所有状态</SelectItem>
                <SelectItem key="enabled">已启用</SelectItem>
                <SelectItem key="disabled">已禁用</SelectItem>
              </Select>
              <span className="text-tiny text-default-400">
                {normalizedQuery || statusFilter !== 'all' ? `筛选后 ${visibleRows.length} 条` : `共 ${pagination.total} 条记录`}
              </span>
            </div>
          </div>

          {error ? (
            <ErrorState error={error} retry={() => load()} />
          ) : (
            <Table aria-label={spec.title} isStriped>
              <TableHeader>
                {[
                  ...visibleFields.map((field) => (
                    <TableColumn key={field.key}>{field.label}</TableColumn>
                  )),
                  <TableColumn key="actions" align="end">操作</TableColumn>,
                ]}
              </TableHeader>
              <TableBody items={visibleRows} emptyContent="暂无数据">
                {(row) => (
                  <TableRow key={entityId(row, spec.idKey)}>
                    {[
                      ...visibleFields.map((field) => (
                        <TableCell key={field.key}>{renderCell(row, field)}</TableCell>
                      )),
                      <TableCell key="actions">
                        <div className="flex items-center justify-end gap-1">
                          {spec.detailPath && (
                            <Button isIconOnly size="sm" variant="light" onPress={() => navigate(`${spec.detailPath}/${entityId(row, spec.idKey)}`)}>
                              <Eye className="w-4 h-4 text-default-500" />
                            </Button>
                          )}
                          {spec.action === 'credit' && (
                            <Button size="sm" variant="flat" color="primary" onPress={() => setActionRow(row)}>充值</Button>
                          )}
                          {!spec.readOnly && (
                            <Button isIconOnly size="sm" variant="light" onPress={() => void openForm(row)}>
                              <Pencil className="w-4 h-4 text-default-500" />
                            </Button>
                          )}
                          {!spec.readOnly && (
                            <Button isIconOnly size="sm" variant="light" color="danger" onPress={() => setConfirmRow(row)}>
                              <Trash2 className="w-4 h-4 text-danger" />
                            </Button>
                          )}
                        </div>
                      </TableCell>,
                    ]}
                  </TableRow>
                )}
              </TableBody>
            </Table>
          )}

          {pagination.total > 0 && (
            <div className="flex items-center justify-between pt-4 border-t border-divider">
              <span className="text-tiny text-default-400">第 {pagination.page} 页，共 {pagination.total_pages || 1} 页</span>
              <Pagination
                total={pagination.total_pages || 1}
                page={pagination.page}
                color="primary"
                size="sm"
                onChange={(page) => load(page)}
              />
            </div>
          )}
        </CardBody>
      </Card>

      <Modal isOpen={editing !== undefined} onOpenChange={(o) => !o && setEditing(undefined)} size="lg">
        <ModalContent>
          <ModalHeader>{isEditing ? `编辑${spec.title}` : `新建${spec.title}`}</ModalHeader>
          <ModalBody>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 py-2">
              {spec.fields
                .filter((field) => !field.readonly && (!field.showWhen || field.showWhen(draft)))
                .map((field) => {
                  const err = validationErrors[field.key];
                  return (
                    <div key={field.key} className={field.fullWidth ? 'md:col-span-2 col-span-1' : 'col-span-1'}>
                      <FieldLabel label={field.label} required={field.required && !(isEditing && field.preserveEmptyOnEdit)} />
                      <FormControl
                        field={field.optionsResource ? { ...field, options: optionsForField(field) } : field}
                        disabled={isEditing && field.key === spec.idKey}
                        value={draft[field.key]}
                        onChange={(value) => updateDraft(field.key, value)}
                      />
                      {err && <p className="text-tiny text-danger mt-1">{err}</p>}
                    </div>
                  );
                })}
            </div>
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onPress={() => setEditing(undefined)}>取消</Button>
            <Button color="primary" isLoading={saving} onPress={save}>保存</Button>
          </ModalFooter>
        </ModalContent>
      </Modal>

      <Modal isOpen={Boolean(actionRow)} onOpenChange={(o) => !o && setActionRow(null)} size="sm">
        <ModalContent>
          <ModalHeader>账户充值 · {actionRow ? entityId(actionRow, spec.idKey) : ''}</ModalHeader>
          <ModalBody>
            <div className="py-2">
              <FieldLabel label="充值金额" required />
              <Input
                type="number"
                variant="bordered"
                min={0.001}
                max={100000000}
                value={String(amount)}
                onValueChange={(v) => setAmount(Number(v) || 0)}
              />
            </div>
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onPress={() => setActionRow(null)}>取消</Button>
            <Button color="primary" isLoading={saving} onPress={runAction}>确认充值</Button>
          </ModalFooter>
        </ModalContent>
      </Modal>

      <ConfirmDialog
        open={Boolean(confirmRow)}
        title="确认删除"
        message="确认删除此资源？该操作不可恢复。"
        loading={saving}
        onConfirm={async () => {
          if (confirmRow) await remove(confirmRow);
          setConfirmRow(null);
        }}
        onClose={() => setConfirmRow(null)}
      />
    </>
  );
}
