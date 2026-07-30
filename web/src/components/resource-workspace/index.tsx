// 资源工作台：通用 CRUD 列表 + 表单 + 分页 + 搜索 + 状态筛选
// 从 console.tsx 拆分

import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Button,
  Card,
  CardBody,
  Input,
  Select,
  SelectItem,
  Pagination,
  Chip,
  Switch,
  Modal,
  ModalContent,
  ModalHeader,
  ModalBody,
  ModalFooter,
  Table,
  TableHeader,
  TableColumn,
  TableBody,
  TableRow,
  TableCell,
  Textarea,
  Tooltip,
} from '@heroui/react';
import {
  Plus,
  RefreshCw,
  Search,
  Eye,
  EyeOff,
  Pencil,
  Trash2,
  Download,
  Upload,
  FileText,
  CheckCircle2,
  Network,
  GitBranch,
  Wallet,
  PhoneCall,
  Settings,
  Cog,
} from 'lucide-react';
import { api } from '@/services/client';
import {
  createResource,
  deleteResource,
  listResource,
  updateResource,
  type Entity,
} from '@/services/resources';
import { listOptions, trunkRole } from '@/services/trunks';
import { EmptyState, ErrorState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import { ExtensionDetailView } from '@/pages/numbers/extension-detail';
import { TrunkDetailView } from '@/pages/trunks/trunk-detail';
import { CallDetailView } from '@/pages/billing/call-detail';
import {
  callDetailText,
  datetimeText,
  entityId,
  valueText,
  moneyText,
  durationSecondsText,
} from '@/pages/shared/format';
import type { FieldSpec, ResourceSpec, SelectOptionSpec } from '@/pages/shared/types';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission } from '@/services/auth';

// customRowAction.icon 字符串 → Lucide 图标组件映射
const ROW_ACTION_ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  Network,
  GitBranch,
  Wallet,
  PhoneCall,
  Settings,
  Cog,
};

const moneyFields = new Set([
  'balance',
  'credit_limit',
  'price_per_interval',
  'amount',
  'balance_after',
  'cost',
  'access_amount',
  'egress_cost',
  'balance_before',
  'access_charge_amount',
  'egress_cost_amount',
]);

/** 按点号分隔的 key 从嵌套对象中取值，例如 getNestedValue(row, 'audit.ingress_trunk_id')。 */
function getNestedValue(source: Entity, key: string): unknown {
  return key.split('.').reduce<unknown>((acc, segment) => {
    if (acc && typeof acc === 'object') {
      return (acc as Record<string, unknown>)[segment];
    }
    return undefined;
  }, source);
}

function resourcePermission(spec: ResourceSpec, action: string): string {
  if (spec.path === '/billing/access-accounts') return `billing.access_accounts.${action}`;
  if (spec.path === '/billing/egress-accounts') return `billing.egress_accounts.${action}`;
  if (spec.path === '/billing/credits') return `billing.credits.${action}`;
  if (spec.path === '/billing/transactions')
    return action === 'export' ? 'billing.ledger.export' : 'billing.ledger.view';
  if (spec.path.startsWith('/routing')) return `routing.${action}`;
  if (spec.path === '/extensions') return `extensions.${action}`;
  if (spec.path === '/numbers') return `numbers.${action}`;
  if (spec.path === '/trunks') return `trunks.${action}`;
  if (spec.path.startsWith('/ivr')) return action === 'view' ? 'ivr.view' : `ivr.${action}`;
  if (spec.path === '/tenants') return `tenants.${action}`;
  if (
    spec.path.includes('caller-pools') ||
    spec.path.includes('egress-groups') ||
    spec.path.includes('did-destinations')
  ) {
    if (action === 'view') return 'termination.view';
    return ['create', 'update', 'delete'].includes(action)
      ? 'termination.manage'
      : `termination.${action}`;
  }
  if (spec.path.startsWith('/security')) {
    if (action === 'view') return 'security.view';
    return ['create', 'update', 'delete'].includes(action)
      ? 'security.manage'
      : `security.${action}`;
  }
  return `${spec.path.replace(/^\//, '').replaceAll('/', '.')}.${action}`;
}

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
  open,
  title,
  message,
  loading,
  onConfirm,
  onClose,
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
          <Button variant="flat" onPress={onClose}>
            取消
          </Button>
          <Button color="danger" isLoading={loading} onPress={onConfirm}>
            确认
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}

export function FormControl({
  field,
  disabled = false,
  value,
  onChange,
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
        step={field.step ?? 'any'}
        placeholder={field.placeholder}
        value={value !== undefined && value !== null ? String(value) : ''}
        onValueChange={(v) => {
          if (v === '' || v === undefined) {
            onChange(undefined);
          } else {
            const num = Number(v);
            onChange(Number.isNaN(num) ? v : num);
          }
        }}
      />
    );
  }
  if (field.kind === 'switch') {
    return (
      <Switch
        isDisabled={disabled}
        isSelected={Boolean(value)}
        onValueChange={(v) => onChange(v)}
      />
    );
  }
  if (field.kind === 'select') {
    const options = (field.options || []).map((option) =>
      typeof option === 'string' ? { label: option, value: option } : option,
    );
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
  if (field.kind === 'multiselect') {
    const options = (field.options || []).map((option) =>
      typeof option === 'string' ? { label: option, value: option } : option,
    );
    const selected = Array.isArray(value)
      ? value.map((item) => String(item))
      : value !== undefined && value !== null && value !== ''
        ? [String(value)]
        : [];
    return (
      <Select
        variant="bordered"
        isDisabled={disabled}
        placeholder={field.placeholder ?? '可选择多项，留空表示暂不关联'}
        selectionMode="multiple"
        selectedKeys={new Set(selected)}
        onSelectionChange={(keys) => {
          if (keys === 'all') {
            onChange(options.map((option) => option.value));
          } else {
            onChange(Array.from(keys));
          }
        }}
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
        autoComplete="new-password"
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
      autoComplete="off"
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
    if (spec.path === '/tenants') {
      // 后端 TenantListItem 使用 #[serde(flatten)]，响应为平铺结构（无嵌套 tenant 对象）
      // billing_account 为展示用对象，在 resourceSaveValues 中会被剔除，不回传后端
      return { ...row };
    }
    if (spec.path === '/billing/access-accounts' || spec.path === '/billing/egress-accounts') {
      // 后端列表返回 trunk_ids（关联中继 ID 数组），表单字段为 gateway_ids
      const trunkIds = Array.isArray(row.trunk_ids) ? row.trunk_ids : [];
      return { ...row, gateway_ids: trunkIds.map((id) => String(id)) };
    }
    return { ...row };
  }
  return spec.fields.reduce<Entity>((defaults, field) => {
    if (field.defaultValue !== undefined) defaults[field.key] = field.defaultValue;
    else if (field.kind === 'switch') defaults[field.key] = false;
    if (
      defaults[field.key] === undefined &&
      field.required &&
      field.kind === 'select' &&
      field.options?.[0]
    ) {
      const option = field.options[0];
      defaults[field.key] = typeof option === 'string' ? option : option.value;
    }
    return defaults;
  }, {});
}

export function resourceSaveValues(spec: ResourceSpec, values: Entity, editing: boolean): Entity {
  const result = { ...values };
  if (spec.path === '/numbers') {
    result.direction = result.can_receive
      ? result.can_present
        ? 'both'
        : 'inbound'
      : result.can_present
        ? 'outbound'
        : 'disabled';
  }
  if ('account_id' in result) {
    if (result.account_id === '' || result.account_id === undefined || result.account_id === null) {
      delete result.account_id;
    } else if (typeof result.account_id === 'string' && !isNaN(Number(result.account_id))) {
      result.account_id = Number(result.account_id);
    }
  }
  if ('billing_account_id' in result) {
    if (
      result.billing_account_id === '' ||
      result.billing_account_id === undefined ||
      result.billing_account_id === null
    ) {
      delete result.billing_account_id;
    } else if (
      typeof result.billing_account_id === 'string' &&
      !isNaN(Number(result.billing_account_id))
    ) {
      result.billing_account_id = Number(result.billing_account_id);
    }
  }
  // tenant_id 空字符串清除关联（后端期望 null 或不传表示全局）
  if ('tenant_id' in result) {
    if (result.tenant_id === '' || result.tenant_id === undefined || result.tenant_id === null) {
      delete result.tenant_id;
    }
  }
  // 移除仅用于展示的关联摘要字段，避免回传后端
  delete result.billing_account_summary;
  delete result.billing_account;
  delete result.associated_tenants_summary;
  delete result.associated_gateways_summary;
  if (['/billing/access-accounts', '/billing/egress-accounts'].includes(spec.path)) {
    delete result.id;
    delete result.balance;
    delete result.account_type;
    delete result.created_at;
    delete result.updated_at;
    // 移除后端列表返回的展示字段，避免回传
    delete result.trunk_id;
    delete result.trunk_ids;
    delete result.tenant_id;
    delete result.name;
    // 确保 gateway_ids 为字符串数组
    if (Array.isArray(result.gateway_ids)) {
      result.gateway_ids = result.gateway_ids
        .map((id) => String(id).trim())
        .filter((id) => id !== '');
    } else if (result.gateway_ids === undefined || result.gateway_ids === null) {
      result.gateway_ids = [];
    }
  }
  if (!editing) return result;
  spec.fields
    .filter((field) => field.kind === 'secret' && field.preserveEmptyOnEdit)
    .forEach((field) => {
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

/** 关键字防抖时长（毫秒） */
const FILTER_DEBOUNCE_MS = 300;

/** 初始化筛选状态：普通筛选按 param 初始化，dateRange 按 `${param}_start`/`${param}_end` 初始化 */
function initFilterState(spec: ResourceSpec): Record<string, string> {
  const state: Record<string, string> = {};
  (spec.serverFilters || []).forEach((f) => {
    if (f.kind === 'dateRange') {
      state[`${f.param}_start`] = '';
      state[`${f.param}_end`] = '';
    } else {
      state[f.param] = '';
    }
  });
  return state;
}

/** 将 server-mode 筛选值整理为后端 query 参数（仅包含非空值） */
function buildServerQueryParams(
  spec: ResourceSpec,
  filters: Record<string, string>,
): Record<string, string> {
  const params: Record<string, string> = {};
  (spec.serverFilters || []).forEach((f) => {
    if (f.mode === 'client') return;
    if (f.kind === 'dateRange') {
      const startKey = `${f.param}_start`;
      const endKey = `${f.param}_end`;
      const startVal = (filters[startKey] ?? '').trim();
      const endVal = (filters[endKey] ?? '').trim();
      // datetime-local 值形如 "2026-07-29T15:30"，补充秒和时区后传给后端
      if (startVal) params[f.startParam ?? startKey] = normalizeDateTimeLocal(startVal, false);
      if (endVal) params[f.endParam ?? endKey] = normalizeDateTimeLocal(endVal, true);
    } else {
      const v = (filters[f.param] ?? '').trim();
      if (v !== '') params[f.param] = v;
    }
  });
  return params;
}

/**
 * 将 `<input type="datetime-local">` 的值（"YYYY-MM-DDTHH:mm"）规范为后端可解析的 RFC3339 时间。
 * - 补全秒（":00"）
 * - 附加本地时区偏移（ Asia/Shanghai => +08:00）
 * - 结束时间（isEnd=true）若未带秒，按当前秒补齐，避免漏掉结束时刻的数据
 */
function normalizeDateTimeLocal(value: string, isEnd: boolean): string {
  // value: "2026-07-29T15:30" 或 "2026-07-29T15:30:45"
  const hasSeconds = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/.test(value);
  let normalized = hasSeconds ? value : `${value}:00`;
  // 附加本地时区偏移：用浏览器当前时区
  const tzOffset = new Date().getTimezoneOffset();
  const sign = tzOffset <= 0 ? '+' : '-';
  const absOffset = Math.abs(tzOffset);
  const tzHours = String(Math.floor(absOffset / 60)).padStart(2, '0');
  const tzMinutes = String(absOffset % 60).padStart(2, '0');
  normalized = `${normalized}${sign}${tzHours}:${tzMinutes}`;
  // 结束时间：若未带秒（用户只选到分钟），秒补为 59，包含该分钟的所有数据
  if (isEnd && !hasSeconds) {
    normalized = normalized.replace(/:\d{2}\+/, ':59+');
    normalized = normalized.replace(/:\d{2}-/, ':59-');
  }
  return normalized;
}

/** 对已加载的行数据应用 client-mode 筛选 */
function applyClientFilters(
  spec: ResourceSpec,
  rows: Entity[],
  filters: Record<string, string>,
): Entity[] {
  const clientFilters = (spec.serverFilters || []).filter((f) => f.mode === 'client');
  if (clientFilters.length === 0) return rows;
  let result = rows;
  for (const f of clientFilters) {
    const raw = (filters[f.param] ?? '').trim();
    if (raw === '') continue;
    const lower = raw.toLowerCase();
    if (f.kind === 'keyword') {
      const fields = f.clientFields && f.clientFields.length > 0 ? f.clientFields : null;
      result = result.filter((row) => {
        if (fields) {
          return fields.some((k) =>
            String(row[k] ?? '')
              .toLowerCase()
              .includes(lower),
          );
        }
        return Object.values(row).some((v) =>
          String(v ?? '')
            .toLowerCase()
            .includes(lower),
        );
      });
    } else {
      // status / select：精确匹配字段值
      result = result.filter((row) => {
        const cell = row[f.param];
        if (typeof cell === 'boolean') return String(cell) === raw;
        return String(cell ?? '') === raw;
      });
    }
  }
  return result;
}

export function ResourceWorkspace({
  spec,
  headerActions,
  headerActionsPermission,
}: {
  spec: ResourceSpec;
  headerActions?: React.ReactNode;
  headerActionsPermission?: string;
}) {
  const { session } = useAuth();
  const may = (action: string) =>
    Boolean(session && hasPermission(session, resourcePermission(spec, action)));
  const [rows, setRows] = useState<Entity[]>([]);
  const [pagination, setPagination] = useState({
    page: 1,
    page_size: 20,
    total: 0,
    total_pages: 0,
  });
  const [filters, setFilters] = useState<Record<string, string>>(() => initFilterState(spec));
  const [debouncedFilters, setDebouncedFilters] = useState<Record<string, string>>(() =>
    initFilterState(spec),
  );
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState<Entity | null | undefined>(undefined);
  const [draft, setDraft] = useState<Entity>({});
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const [actionRow, setActionRow] = useState<Entity | null>(null);
  const [actionIdempotencyKey, setActionIdempotencyKey] = useState('');
  const [amount, setAmount] = useState<number>(100);
  const [creditRemark, setCreditRemark] = useState('');
  const [fieldOptions, setFieldOptions] = useState<Record<string, SelectOptionSpec[]>>({});
  const [confirmRow, setConfirmRow] = useState<Entity | null>(null);
  const [detailModalRow, setDetailModalRow] = useState<Entity | null>(null);
  const [isImportOpen, setIsImportOpen] = useState(false);
  const [importFile, setImportFile] = useState<File | null>(null);
  const [importing, setImporting] = useState(false);

  // server-mode 筛选参数（仅非空值，传给后端）
  const serverParams = useMemo(
    () => buildServerQueryParams(spec, debouncedFilters),
    [spec, debouncedFilters],
  );

  const load = useCallback(
    async (page = pagination.page) => {
      setLoading(true);
      setError('');
      try {
        const baseParams = {
          page,
          page_size: pagination.page_size,
          ...spec.params,
          ...serverParams,
        };
        if (spec.path === '/extensions') {
          const [result, regRes, sysRes, tenantsRes] = await Promise.all([
            listResource(spec.path, baseParams),
            api.get<{ items: Entity[] }>('/registrations').catch(() => ({ items: [] as Entity[] })),
            api
              .get<{ configs?: Record<string, string> }>('/infrastructure/settings')
              .catch(() => ({ configs: {} })),
            api.get<{ items: Entity[] }>('/tenants').catch(() => ({ items: [] as Entity[] })),
          ]);
          const onlineAors = new Set((regRes.items || []).map((r) => String(r.aor ?? '')));
          const sysRealm = (sysRes?.configs as Record<string, string>)?.realm || 'vos-rs (默认)';
          const tenantMap = new Map<string, string>(
            (tenantsRes.items || []).map((t) => [String(t.id), String(t.domain || t.name || '')]),
          );
          const items = (result.items || []).map((user) => {
            const u = String(user.username ?? '');
            const isOnline =
              onlineAors.has(u) ||
              Array.from(onlineAors).some(
                (aor) => aor.includes(`:${u}@`) || aor.includes(`:${u};`) || aor.endsWith(`:${u}`),
              );
            const tenantId = user.tenant_id ? String(user.tenant_id) : '';
            const tenantDomain = tenantId ? tenantMap.get(tenantId) : undefined;
            const displayRealm = user.realm
              ? String(user.realm)
              : tenantDomain
                ? tenantDomain
                : sysRealm;
            return {
              ...user,
              sip_domain: user.sip_domain || '127.0.0.1:5060',
              realm: displayRealm,
              registration_status: isOnline ? 'registered' : 'unregistered',
            };
          });
          const filtered = applyClientFilters(spec, items, debouncedFilters);
          setRows(filtered);
          setPagination(
            result.pagination || { page, page_size: 20, total: filtered.length, total_pages: 1 },
          );
        } else if (spec.path === '/tenants') {
          // 租户列表：后端 TenantListItem 使用 #[serde(flatten)]，响应为平铺结构。
          // billing_summary 为关联对接账户的聚合摘要对象，直接保留供 renderCell 渲染。
          const result = await listResource(spec.path, baseParams);
          const items = result.items || [];
          const filtered = applyClientFilters(spec, items, debouncedFilters);
          setRows(filtered);
          setPagination(
            result.pagination || { page, page_size: 20, total: filtered.length, total_pages: 1 },
          );
        } else if (['/billing/access-accounts', '/billing/egress-accounts'].includes(spec.path)) {
          // 计费账户列表：后端返回 associated_tenants 数组，平铺为展示字段
          const result = await listResource(spec.path, baseParams);
          const items = (result.items || []).map((row) => {
            const tenants = Array.isArray(row.associated_tenants)
              ? (row.associated_tenants as Entity[])
              : [];
            const gateways = Array.isArray(row.associated_gateways)
              ? (row.associated_gateways as Entity[])
              : [];
            const gatewayId = row.gateway_id ?? gateways[0]?.id;
            return {
              ...row,
              gateway_id: gatewayId,
              associated_tenants_summary:
                tenants.length > 0
                  ? tenants.map((t) => String(t.name ?? t.id ?? '')).join('、')
                  : '—',
              associated_gateways_summary:
                gateways.length > 0
                  ? gateways.map((gateway) => String(gateway.name ?? gateway.id ?? '')).join('、')
                  : valueText(gatewayId),
            };
          });
          const filtered = applyClientFilters(spec, items, debouncedFilters);
          setRows(filtered);
          setPagination(
            result.pagination || { page, page_size: 20, total: filtered.length, total_pages: 1 },
          );
        } else if (['/billing/transactions', '/billing/credits'].includes(spec.path)) {
          const result = await listResource(spec.path, baseParams);
          const items = applyClientFilters(
            spec,
            (result.items || []).map((row) => ({
              ...row,
              created_at: row.created_at ?? row.occurred_at,
              transaction_type: row.transaction_type ?? row.entry_type ?? 'call_charge',
            })),
            debouncedFilters,
          );
          setRows(items);
          setPagination(
            result.pagination || { page, page_size: 20, total: items.length, total_pages: 1 },
          );
        } else {
          const result = await listResource(spec.path, baseParams);
          const items = applyClientFilters(spec, result.items || [], debouncedFilters);
          setRows(items);
          // 客户端筛选模式下，total 用筛选后的条数（后端通常无分页）
          const isClientMode = (spec.serverFilters || []).some((f) => f.mode === 'client');
          const fallbackTotal = isClientMode ? items.length : result.items?.length || 0;
          setPagination(
            result.pagination || { page, page_size: 20, total: fallbackTotal, total_pages: 1 },
          );
        }
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : '加载失败');
      } finally {
        setLoading(false);
      }
    },
    [pagination.page, pagination.page_size, spec.path, spec.params, serverParams],
  );

  // 关键字防抖：filters 变化后延迟同步到 debouncedFilters
  useEffect(() => {
    const t = setTimeout(() => setDebouncedFilters(filters), FILTER_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [filters]);

  // 切换资源或筛选条件变化时回到第 1 页并触发加载
  const hasServerFilter = useMemo(
    () => Object.values(serverParams).some((v) => v !== ''),
    [serverParams],
  );
  const hasClientFilter = useMemo(() => {
    const clientFilters = (spec.serverFilters || []).filter((f) => f.mode === 'client');
    return clientFilters.some((f) => (filters[f.param] ?? '').trim() !== '');
  }, [filters, spec]);

  const exportData = async () => {
    if (!rows.length) {
      message.warning('当前列表无数据可导出');
      return;
    }
    try {
      setLoading(true);
      const queryParams = new URLSearchParams({
        export: 'true',
        ...spec.params,
        ...serverParams,
      }).toString();

      const blob = await api.blob(`${spec.path}?${queryParams}`);
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.setAttribute('href', url);
      const filename = `${spec.title}_${new Date().toISOString().slice(0, 10)}.csv`;

      link.setAttribute('download', filename);
      link.style.visibility = 'hidden';
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      message.success(`已从后端成功生成并下载全部数据 (CSV 格式)`);
    } catch (err) {
      message.error(err instanceof Error ? err.message : '请求后端导出数据失败');
    } finally {
      setLoading(false);
    }
  };

  const downloadTemplate = async () => {
    try {
      setLoading(true);
      const blob = await api.blob(`${spec.path}/import-template`);
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.setAttribute('href', url);
      const filename = `${spec.title}_导入模板.csv`;
      link.setAttribute('download', filename);
      link.style.visibility = 'hidden';
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      message.success('导入模板下载成功');
    } catch (err) {
      message.error(err instanceof Error ? err.message : '下载导入模板失败');
    } finally {
      setLoading(false);
    }
  };

  const handleImportSubmit = async () => {
    if (!importFile) {
      message.warning('请先选择要上传的 CSV 文件');
      return;
    }
    try {
      setImporting(true);
      const formData = new FormData();
      formData.append('file', importFile);
      const response = (await api.post(`${spec.path}/import`, formData, {
        headers: {
          'Content-Type': 'multipart/form-data',
        },
      })) as any;
      const count = response.data?.imported_count || 0;
      message.success(`成功导入 ${count} 条记录！`);
      setIsImportOpen(false);
      setImportFile(null);
      void load(1);
    } catch (err: any) {
      const errMsg = err.response?.data?.message || err.message || '导入数据失败';
      message.error(errMsg);
    } finally {
      setImporting(false);
    }
  };

  // 切换资源时立即重置筛选状态（debouncedFilters 同步重置，跳过防抖）
  useEffect(() => {
    const empty = initFilterState(spec);
    setFilters(empty);
    setDebouncedFilters(empty);
  }, [spec]);

  // 筛选条件变化（含资源切换后的重置）时回到第 1 页并触发加载
  useEffect(() => {
    setPagination((prev) => ({ ...prev, page: 1 }));
    void load(1);
  }, [debouncedFilters, load]);

  useEffect(() => {
    const needsEgress = spec.fields.some((field) => field.optionsResource === 'egress-trunks');
    const needsAccess = spec.fields.some((field) => field.optionsResource === 'access-trunks');
    const needsSource = spec.fields.some((field) => field.optionsResource === 'allocation-source');
    const needsAccessAccounts = spec.fields.some(
      (field) => field.optionsResource === 'access-accounts',
    );
    const needsEgressAccounts = spec.fields.some(
      (field) => field.optionsResource === 'egress-accounts',
    );
    const needsTenants = spec.fields.some((field) => field.optionsResource === 'tenants');
    // 筛选器也需要加载动态选项（tenants / accounts）
    const filterNeedsTenants = (spec.serverFilters || []).some(
      (f) => f.optionsResource === 'tenants',
    );
    const filterNeedsAccounts = (spec.serverFilters || []).some(
      (f) => f.optionsResource === 'accounts',
    );
    const wantEgress = needsEgress;
    const wantAccess = needsAccess;
    const wantSource = needsSource;
    const wantAccounts = needsAccessAccounts || needsEgressAccounts || filterNeedsAccounts;
    const accountsPath = needsEgressAccounts
      ? '/billing/egress-accounts'
      : '/billing/access-accounts';
    const wantTenants = needsTenants || filterNeedsTenants;
    if (!wantEgress && !wantAccess && !wantSource && !wantAccounts && !wantTenants) return;
    void Promise.all([
      wantEgress || wantAccess || wantSource ? listOptions('/trunks') : Promise.resolve([]),
      wantSource ? listOptions('/extensions') : Promise.resolve([]),
      wantAccounts ? listOptions(accountsPath) : Promise.resolve([]),
      wantTenants ? listOptions('/tenants') : Promise.resolve([]),
    ])
      .then(([trunks, extensions, accounts, tenants]) =>
        setFieldOptions({
          owner_egress_trunk_id: trunks
            .filter((item) => trunkRole(item) === 'egress')
            .map((item) => ({ label: String(item.id), value: String(item.id) })),
          allocation_trunks: trunks
            .filter((item) => trunkRole(item) === 'access')
            .map((item) => ({ label: String(item.id), value: String(item.id) })),
          allocation_extensions: extensions.map((item) => ({
            label: String(item.display_name ?? item.username),
            value: String(item.username),
          })),
          account_id: accounts.map((acc) => ({
            label: `${acc.username} (余额: ¥${acc.balance ?? 0})`,
            value: String(acc.username),
          })),
          billing_account_id: accounts.map((acc) => ({
            label: `${acc.username} (余额: ¥${acc.balance ?? 0})`,
            value: String(acc.id),
          })),
          // 筛选器用账户列表（value=username，便于按 username 精确过滤流水）
          username: accounts.map((acc) => ({
            label: `${acc.username} (余额: ¥${acc.balance ?? 0})`,
            value: String(acc.username),
          })),
          tenant_id: tenants.map((item) => {
            // 后端 TenantListItem 使用 #[serde(flatten)]，响应为平铺结构
            return {
              label: String(item.name ?? item.id ?? ''),
              value: String(item.id ?? ''),
            };
          }),
          gateway_ids: trunks
            .filter((item) =>
              wantAccess
                ? trunkRole(item) === 'access' && Boolean(item.tenant_id)
                : trunkRole(item) === 'egress',
            )
            .map((item) => ({
              label: String(item.name ?? item.id),
              value: String(item.id),
            })),
        }),
      )
      .catch(() =>
        setFieldOptions({
          owner_egress_trunk_id: [],
          allocation_trunks: [],
          allocation_extensions: [],
          account_id: [],
          billing_account_id: [],
          username: [],
          tenant_id: [],
          gateway_ids: [],
        }),
      );
  }, [spec.path]);

  const optionsForField = (field: FieldSpec) => {
    if (field.optionsResource === 'allocation-source') {
      const sourceType =
        field.key === 'owner_source_id' ? draft.owner_source_type : draft.allocation_source_type;
      return sourceType === 'trunk'
        ? fieldOptions.allocation_trunks || []
        : fieldOptions.allocation_extensions || [];
    }
    return field.optionsResource ? fieldOptions[field.key] || [] : field.options;
  };

  const isEditing = editing !== undefined && editing !== null;
  const openForm = async (row: Entity | null) => {
    let values = resourceFormValues(spec, row);
    if (row && spec.path === '/numbers') {
      try {
        const allocations = await api.get<Entity[]>(
          `/numbers/${encodeURIComponent(entityId(row, spec.idKey))}/allocations`,
        );
        const active = allocations.find((allocation) => allocation.enabled !== false);
        if (active)
          values = {
            ...values,
            allocation_source_type: active.source_type,
            allocation_source_id: active.source_id,
          };
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
        else if (!isEmpty && field.pattern && !field.pattern.test(String(value)))
          result[field.key] = field.patternMessage || `${field.label}格式不正确`;
        else if (!isEmpty && field.min !== undefined && Number(value) < field.min)
          result[field.key] = `${field.label}不能小于 ${field.min}`;
        return result;
      }, {});
      if (Object.keys(errors).length) {
        setValidationErrors(errors);
        return;
      }
      const values = { ...resourceSaveValues(spec, draft, isEditing), ...spec.params };
      const allocation =
        spec.path === '/numbers'
          ? {
              source_type: String(values.allocation_source_type ?? ''),
              source_id: String(values.allocation_source_id ?? ''),
              enabled: true,
            }
          : null;
      delete values.allocation_source_type;
      delete values.allocation_source_id;
      setSaving(true);
      if (isEditing)
        await updateResource(spec.path, entityId(editing as Entity, spec.idKey), values);
      else await createResource(spec.path, values);
      if (allocation) {
        const number = isEditing ? entityId(editing as Entity, spec.idKey) : String(values.number);
        await api.put(`/numbers/${encodeURIComponent(number)}/allocations`, {
          items: [allocation],
        });
      }
      message.success(isEditing ? '已保存更改' : '已创建');
      setEditing(undefined);
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '操作失败');
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
      await api.post(
        `${spec.path}/${encodeURIComponent(entityId(actionRow, spec.idKey))}/credit`,
        { amount, remark: creditRemark.trim() },
        { headers: { 'Idempotency-Key': actionIdempotencyKey } },
      );
      message.success('充值成功');
      setActionRow(null);
      setActionIdempotencyKey('');
      setCreditRemark('');
      await load();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : '操作失败');
    } finally {
      setSaving(false);
    }
  };

  // rows 已在 load() 中完成服务端/客户端筛选，直接作为可见行
  const visibleRows = rows;
  const hasActiveFilter = hasServerFilter || hasClientFilter;

  const visibleFields = spec.fields
    .filter(
      (field) =>
        !field.tableHidden && (!field.showWhen || rows.some((row) => field.showWhen!(row))),
    )
    .slice(0, spec.tableFieldLimit ?? 7);

  const [visibleSecrets, setVisibleSecrets] = useState<Record<string, boolean>>({});
  const toggleSecretVisibility = (rowKey: string, fieldKey: string) => {
    const key = `${rowKey}:${fieldKey}`;
    setVisibleSecrets((prev) => ({ ...prev, [key]: !prev[key] }));
  };

  const renderCell = (row: Entity, field: FieldSpec) => {
    let value = field.key.includes('.') ? getNestedValue(row, field.key) : row[field.key];
    // 租户计费摘要：后端返回 billing_summary 对象，展示总余额、账户数与欠费状态
    if (field.key === 'billing_summary') {
      const summary = value as
        | {
            total_balance?: number | string;
            overdue_count?: number;
            account_count?: number;
            status?: string;
          }
        | undefined;
      if (!summary || !summary.account_count) {
        return <span className="text-default-400">未关联账户</span>;
      }
      const status = summary.status ?? 'normal';
      const color = status === 'overdue' ? 'danger' : status === 'normal' ? 'success' : 'default';
      const balance = Number(summary.total_balance ?? 0).toFixed(2);
      return (
        <div className="flex flex-col gap-1">
          <span className="font-mono text-tiny">余额 ¥{balance}</span>
          <span className="text-tiny text-default-400">
            {summary.account_count} 个账户
            {summary.overdue_count ? ` · ${summary.overdue_count} 个欠费` : ''}
          </span>
          <Chip size="sm" variant="flat" color={color}>
            {status === 'overdue' ? '欠费' : status === 'normal' ? '正常' : '无账户'}
          </Chip>
        </div>
      );
    }
    if (field.key === 'registration_status') {
      const registered = value === 'registered';
      return (
        <Chip
          size="sm"
          color={registered ? 'success' : 'default'}
          variant="flat"
          startContent={
            <span
              className={`w-1.5 h-1.5 rounded-full ${registered ? 'bg-success animate-pulse' : 'bg-default-400'}`}
            />
          }
        >
          {registered ? '已注册/在线' : '未注册/离线'}
        </Chip>
      );
    }
    if (field.key === 'node_count') {
      value = Array.isArray(row.nodes) ? row.nodes.length : (row.node_count ?? 0);
    }
    if (field.key === 'tenant_id') {
      // 所属商户：空值显示"全局费率"（费率）或"全局"（分机），有值显示商户名称
      if (value === undefined || value === null || value === '') {
        return <span className="text-default-400">全局</span>;
      }
      const tenantOptions = fieldOptions.tenant_id || [];
      const matched = tenantOptions.find((option) => option.value === String(value));
      return <span className="text-default-600">{matched?.label ?? valueText(value)}</span>;
    }
    const callText = spec.path === '/calls' ? callDetailText(value, field.key) : undefined;
    if (['status', 'state', 'enabled', 'health'].includes(field.key)) {
      const positive = [
        'active',
        'online',
        'registered',
        'healthy',
        'answered',
        'enabled',
        'closed',
        true,
      ].includes(value as never);
      return (
        <Chip size="sm" color={positive ? 'success' : 'danger'} variant="flat">
          {callText ?? (typeof value === 'boolean' ? (value ? '启用' : '停用') : valueText(value))}
        </Chip>
      );
    }
    if (field.kind === 'secret') {
      const rowId = entityId(row, spec.idKey);
      const secretKey = `${rowId}:${field.key}`;
      const isVisible = Boolean(visibleSecrets[secretKey]);
      const rawText = String(value ?? '');
      return (
        <div className="flex items-center gap-1.5 font-mono text-tiny">
          <span>{isVisible ? rawText || '—' : '••••••••'}</span>
          {rawText && (
            <Button
              isIconOnly
              size="sm"
              variant="light"
              className="w-6 h-6 min-w-6 text-default-400 hover:text-default-600"
              onPress={() => toggleSecretVisibility(rowId, field.key)}
              aria-label={isVisible ? '隐藏密码' : '显示密码'}
            >
              {isVisible ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
            </Button>
          )}
        </div>
      );
    }
    let text: string;
    if (callText) text = callText;
    else if (field.kind === 'duration') text = durationSecondsText(value);
    else if (field.kind === 'datetime') text = datetimeText(value);
    else if (moneyFields.has(field.key)) text = moneyText(value);
    else if (Array.isArray(value)) {
      // 数组字段（如 trunk_ids）展示为逗号分隔的列表
      const items = value.map((item) => String(item)).filter((item) => item !== '');
      text = items.length > 0 ? items.join('、') : '—';
    } else if (field.kind === 'select') {
      const options = (field.options || fieldOptions[field.key] || []).map((option) =>
        typeof option === 'string' ? { label: option, value: option } : option,
      );
      const actual = field.key === 'role' ? trunkRole(row) : value;
      text = valueText(
        options.find((option) => option.value === String(actual))?.label ??
          (field.key === 'role' ? (trunkRole(row) === 'access' ? '接入中继' : '落地中继') : value),
      );
    } else text = valueText(value);
    return (
      <span
        className={
          field.key.includes('id') || field.key.includes('number')
            ? 'font-mono text-foreground'
            : 'text-default-600'
        }
      >
        {text}
      </span>
    );
  };

  return (
    <>
      <Card shadow="none" className="overview-card p-2">
        <CardBody className="p-4 flex flex-col gap-4">
          <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-divider">
            <div>
              <h2 className="text-base font-semibold text-foreground">{spec.title}</h2>
              {spec.description && (
                <p className="text-tiny text-default-500 mt-0.5">{spec.description}</p>
              )}
            </div>
            <div className="flex items-center gap-2">
              {headerActions &&
                Boolean(
                  session &&
                  hasPermission(
                    session,
                    headerActionsPermission ?? resourcePermission(spec, 'update'),
                  ),
                ) &&
                headerActions}
              <Button
                variant="flat"
                size="sm"
                isLoading={loading}
                onPress={() => load()}
                startContent={<RefreshCw className="w-4 h-4" />}
              >
                刷新
              </Button>
              {may('export') && (
                <Button
                  variant="flat"
                  size="sm"
                  onPress={exportData}
                  startContent={<Download className="w-4 h-4" />}
                >
                  导出
                </Button>
              )}
              {may('import') &&
                ['/extensions', '/numbers', '/routing/rules'].includes(spec.path) && (
                  <Button
                    variant="flat"
                    size="sm"
                    onPress={() => setIsImportOpen(true)}
                    startContent={<Upload className="w-4 h-4" />}
                  >
                    导入
                  </Button>
                )}
              {!spec.readOnly && may('create') && (
                <Button
                  color="primary"
                  size="sm"
                  onPress={() => void openForm(null)}
                  startContent={<Plus className="w-4 h-4" />}
                >
                  {spec.createLabel || '新建'}
                </Button>
              )}
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex flex-wrap items-center gap-3">
              {(spec.serverFilters || []).map((f) => {
                if (f.kind === 'keyword') {
                  return (
                    <Input
                      key={f.param}
                      type="search"
                      placeholder={f.placeholder || `按${f.label}搜索...`}
                      variant="bordered"
                      size="sm"
                      className="w-56"
                      startContent={<Search className="w-4 h-4 text-default-400" />}
                      value={filters[f.param] ?? ''}
                      onValueChange={(v) => setFilters((prev) => ({ ...prev, [f.param]: v }))}
                      isClearable
                      onClear={() => setFilters((prev) => ({ ...prev, [f.param]: '' }))}
                      autoComplete="none"
                      name={`filter_${f.param}`}
                    />
                  );
                }
                if (f.kind === 'dateRange') {
                  const startKey = `${f.param}_start`;
                  const endKey = `${f.param}_end`;
                  return (
                    <div key={f.param} className="flex items-center gap-1.5">
                      <input
                        type="datetime-local"
                        aria-label={`${f.label}开始`}
                        className="h-8 w-44 rounded-lg border border-default-200 bg-content1 px-2 text-tiny text-foreground outline-none focus:border-primary"
                        value={filters[startKey] ?? ''}
                        onChange={(e) =>
                          setFilters((prev) => ({ ...prev, [startKey]: e.target.value }))
                        }
                      />
                      <span className="text-tiny text-default-400">至</span>
                      <input
                        type="datetime-local"
                        aria-label={`${f.label}结束`}
                        className="h-8 w-44 rounded-lg border border-default-200 bg-content1 px-2 text-tiny text-foreground outline-none focus:border-primary"
                        value={filters[endKey] ?? ''}
                        onChange={(e) =>
                          setFilters((prev) => ({ ...prev, [endKey]: e.target.value }))
                        }
                      />
                    </div>
                  );
                }
                // status / select 类型：下拉选项
                const opts =
                  f.optionsResource === 'tenants'
                    ? fieldOptions.tenant_id || []
                    : f.optionsResource === 'accounts'
                      ? fieldOptions.username || []
                      : f.options || [];
                const allOpts = [
                  { label: '全部', value: '' },
                  ...opts.filter((o) => o.value !== ''),
                ];
                const sel = filters[f.param] ?? '';
                return (
                  <Select
                    key={f.param}
                    aria-label={f.label}
                    variant="bordered"
                    size="sm"
                    className="w-36"
                    placeholder={f.label}
                    selectedKeys={sel !== '' ? [sel] : ['']}
                    onChange={(e) => setFilters((prev) => ({ ...prev, [f.param]: e.target.value }))}
                  >
                    {allOpts.map((o) => (
                      <SelectItem key={o.value}>{o.label}</SelectItem>
                    ))}
                  </Select>
                );
              })}
              {(spec.serverFilters || []).length === 0 && (
                <span className="text-tiny text-default-400">该资源暂不支持筛选</span>
              )}
              <span className="text-tiny text-default-400">
                {hasActiveFilter
                  ? `筛选后 ${visibleRows.length} 条`
                  : `共 ${pagination.total} 条记录`}
              </span>
              {hasActiveFilter && (
                <Button
                  size="sm"
                  variant="flat"
                  color="primary"
                  onPress={() => {
                    const empty = initFilterState(spec);
                    setFilters(empty);
                    setDebouncedFilters(empty);
                  }}
                >
                  重置筛选
                </Button>
              )}
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
                  <TableColumn key="actions" align="end">
                    操作
                  </TableColumn>,
                ]}
              </TableHeader>
              <TableBody
                items={visibleRows}
                emptyContent={
                  <EmptyState
                    icon={Search}
                    title={hasActiveFilter ? '没有匹配的数据' : '暂无数据'}
                    description={
                      hasActiveFilter ? '调整筛选条件后再试' : `暂时没有可展示的${spec.title}`
                    }
                    action={
                      hasActiveFilter ? (
                        <Button
                          size="sm"
                          variant="flat"
                          onPress={() => {
                            const empty = initFilterState(spec);
                            setFilters(empty);
                            setDebouncedFilters(empty);
                          }}
                        >
                          清除筛选
                        </Button>
                      ) : undefined
                    }
                  />
                }
              >
                {(row) => (
                  <TableRow key={entityId(row, spec.idKey)}>
                    {[
                      ...visibleFields.map((field) => (
                        <TableCell key={field.key}>{renderCell(row, field)}</TableCell>
                      )),
                      <TableCell key="actions">
                        <div className="flex items-center justify-end gap-1">
                          {spec.customRowAction && may('update') && (
                            <Tooltip
                              content={spec.customRowAction.label}
                              placement="top"
                              delay={300}
                            >
                              <Button
                                isIconOnly
                                size="sm"
                                color={spec.customRowAction.color ?? 'primary'}
                                variant="flat"
                                onPress={() => spec.customRowAction!.onPress(row)}
                                aria-label={spec.customRowAction.label}
                              >
                                {(() => {
                                  const Icon = spec.customRowAction!.icon
                                    ? ROW_ACTION_ICONS[spec.customRowAction!.icon]
                                    : undefined;
                                  return Icon ? (
                                    <Icon className="w-4 h-4" />
                                  ) : (
                                    <Network className="w-4 h-4" />
                                  );
                                })()}
                              </Button>
                            </Tooltip>
                          )}
                          {spec.detailPath && (
                            <Tooltip content="查看详情" placement="top" delay={300}>
                              <Button
                                isIconOnly
                                size="sm"
                                variant="light"
                                onPress={() => setDetailModalRow(row)}
                                aria-label="查看详情"
                              >
                                <Eye className="w-4 h-4 text-default-500" />
                              </Button>
                            </Tooltip>
                          )}
                          {spec.action === 'credit' && may('credit') && (
                            <Tooltip content="充值" placement="top" delay={300}>
                              <Button
                                isIconOnly
                                size="sm"
                                variant="flat"
                                color="primary"
                                onPress={() => {
                                  setActionIdempotencyKey(crypto.randomUUID());
                                  setCreditRemark('');
                                  setActionRow(row);
                                }}
                                aria-label="充值"
                              >
                                <Wallet className="w-4 h-4" />
                              </Button>
                            </Tooltip>
                          )}
                          {!spec.readOnly && may('update') && (
                            <Tooltip content="编辑" placement="top" delay={300}>
                              <Button
                                isIconOnly
                                size="sm"
                                variant="light"
                                onPress={() => void openForm(row)}
                                aria-label="编辑"
                              >
                                <Pencil className="w-4 h-4 text-default-500" />
                              </Button>
                            </Tooltip>
                          )}
                          {!spec.readOnly && may('delete') && (
                            <Tooltip content="删除" placement="top" delay={300}>
                              <Button
                                isIconOnly
                                size="sm"
                                variant="light"
                                color="danger"
                                onPress={() => setConfirmRow(row)}
                                aria-label="删除"
                              >
                                <Trash2 className="w-4 h-4 text-danger" />
                              </Button>
                            </Tooltip>
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
              <span className="text-tiny text-default-400">
                第 {pagination.page} 页，共 {pagination.total_pages || 1} 页
              </span>
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

      <Modal
        isOpen={editing !== undefined}
        onOpenChange={(o) => !o && setEditing(undefined)}
        size="lg"
      >
        <ModalContent>
          <ModalHeader>{isEditing ? `编辑${spec.title}` : `新建${spec.title}`}</ModalHeader>
          <ModalBody>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 py-2">
              {spec.fields
                .filter(
                  (field) =>
                    !field.formHidden &&
                    !field.readonly &&
                    (!field.showWhen || field.showWhen(draft)),
                )
                .map((field) => {
                  const err = validationErrors[field.key];
                  return (
                    <div
                      key={field.key}
                      className={field.fullWidth ? 'md:col-span-2 col-span-1' : 'col-span-1'}
                    >
                      <FieldLabel
                        label={field.label}
                        required={field.required && !(isEditing && field.preserveEmptyOnEdit)}
                      />
                      <FormControl
                        field={
                          field.optionsResource
                            ? { ...field, options: optionsForField(field) }
                            : field
                        }
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
            <Button variant="flat" onPress={() => setEditing(undefined)}>
              取消
            </Button>
            <Button color="primary" isLoading={saving} onPress={save}>
              保存
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>

      <Modal
        isOpen={Boolean(actionRow)}
        onOpenChange={(open) => {
          if (!open) {
            setActionRow(null);
            setActionIdempotencyKey('');
            setCreditRemark('');
          }
        }}
        size="sm"
      >
        <ModalContent>
          <ModalHeader>账户充值 · {actionRow ? entityId(actionRow, spec.idKey) : ''}</ModalHeader>
          <ModalBody>
            <div className="py-2">
              <FieldLabel label="充值金额" required />
              <Input
                type="number"
                variant="bordered"
                min={0.001}
                step="any"
                max={100000000}
                value={String(amount)}
                onValueChange={(v) => setAmount(v === '' ? ('' as unknown as number) : Number(v))}
              />
              <div className="mt-4">
                <FieldLabel label="充值说明" />
                <Textarea
                  variant="bordered"
                  minRows={2}
                  maxRows={4}
                  placeholder="填写本次充值的业务说明"
                  value={creditRemark}
                  onValueChange={setCreditRemark}
                />
              </div>
            </div>
          </ModalBody>
          <ModalFooter>
            <Button
              variant="flat"
              onPress={() => {
                setActionRow(null);
                setActionIdempotencyKey('');
                setCreditRemark('');
              }}
            >
              取消
            </Button>
            <Button color="primary" isLoading={saving} onPress={runAction}>
              确认充值
            </Button>
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

      <Modal
        isOpen={Boolean(detailModalRow)}
        onOpenChange={(open) => !open && setDetailModalRow(null)}
        size="4xl"
        scrollBehavior="inside"
      >
        <ModalContent
          style={
            spec.path === '/calls'
              ? { height: 'min(820px, calc(100vh - 48px))', maxHeight: 'none' }
              : undefined
          }
        >
          <ModalHeader className="text-lg font-bold border-b border-divider flex items-center gap-2">
            <span>
              {spec.title}详情 · {detailModalRow ? entityId(detailModalRow, spec.idKey) : ''}
            </span>
          </ModalHeader>
          <ModalBody className="p-6 flex-1 overflow-y-auto">
            {detailModalRow &&
              (spec.path === '/extensions' ? (
                <ExtensionDetailView id={entityId(detailModalRow, spec.idKey)} />
              ) : spec.path === '/calls' ? (
                <CallDetailView id={entityId(detailModalRow, spec.idKey)} />
              ) : (
                <TrunkDetailView id={entityId(detailModalRow, spec.idKey)} />
              ))}
          </ModalBody>
          <ModalFooter className="border-t border-divider">
            <Button color="primary" variant="flat" onPress={() => setDetailModalRow(null)}>
              关闭
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>

      <Modal
        isOpen={isImportOpen}
        onOpenChange={(open) => {
          if (!open) {
            setIsImportOpen(false);
            setImportFile(null);
          }
        }}
        size="2xl"
      >
        <ModalContent>
          <ModalHeader className="text-lg font-bold border-b border-divider">
            批量数据导入 · {spec.title}
          </ModalHeader>
          <ModalBody className="py-5">
            <div className="flex flex-col gap-5">
              {/* 第一步：下载模板 */}
              <div className="flex items-center justify-between p-4 bg-content2 rounded-xl border border-divider">
                <div className="flex items-start gap-3">
                  <div className="w-10 h-10 rounded-lg bg-primary/10 text-primary flex items-center justify-center shrink-0">
                    <FileText className="w-5 h-5" />
                  </div>
                  <div>
                    <h4 className="text-small font-bold text-foreground">
                      第一步：获取导入数据模板
                    </h4>
                    <p className="text-tiny text-default-500 mt-0.5">
                      使用官方模板避免列名或格式不匹配。
                    </p>
                  </div>
                </div>
                <Button size="sm" color="primary" variant="flat" onPress={downloadTemplate}>
                  下载模板
                </Button>
              </div>

              {/* 第二步：上传 CSV */}
              <div className="flex flex-col gap-2.5">
                <h4 className="text-small font-bold text-foreground">
                  第二步：选择并上传 CSV 文件
                </h4>
                <div
                  className="flex flex-col items-center justify-center gap-3 border-2 border-dashed border-default-300 hover:border-primary hover:bg-primary/5 rounded-2xl py-12 px-6 bg-content2/40 cursor-pointer transition-all"
                  onClick={() => document.getElementById('csv-import-file')?.click()}
                >
                  <div className="w-14 h-14 rounded-2xl bg-primary/10 text-primary flex items-center justify-center">
                    <Upload className="w-7 h-7" />
                  </div>
                  {importFile ? (
                    <div className="flex flex-col items-center gap-1">
                      <span className="text-base font-bold text-success flex items-center gap-1.5">
                        <CheckCircle2 className="w-4 h-4" />
                        {importFile.name}
                      </span>
                      <span className="text-tiny text-default-500">
                        {(importFile.size / 1024).toFixed(1)} KB · 点击重新选择
                      </span>
                    </div>
                  ) : (
                    <div className="flex flex-col items-center gap-1">
                      <span className="text-base font-medium text-foreground">
                        点击选择或拖拽 CSV 文件至此
                      </span>
                      <span className="text-tiny text-default-500">仅支持 .csv 格式文件</span>
                    </div>
                  )}
                  <input
                    id="csv-import-file"
                    type="file"
                    accept=".csv"
                    className="hidden"
                    onChange={(e) => {
                      if (e.target.files?.[0]) {
                        setImportFile(e.target.files[0]);
                      }
                    }}
                  />
                </div>
              </div>
            </div>
          </ModalBody>
          <ModalFooter className="border-t border-divider">
            <Button
              variant="flat"
              onPress={() => {
                setIsImportOpen(false);
                setImportFile(null);
              }}
            >
              取消
            </Button>
            <Button
              color="primary"
              isLoading={importing}
              onPress={handleImportSubmit}
              isDisabled={!importFile}
              startContent={!importing && <Upload className="w-4 h-4" />}
            >
              验证并导入
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </>
  );
}
