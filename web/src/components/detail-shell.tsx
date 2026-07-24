import { useCallback, useState, type ReactNode } from 'react';
import { Button, Card, CardBody, Chip, Spinner } from '@heroui/react';
import { Plus, RefreshCw, Save, type LucideIcon } from 'lucide-react';

/** 详情页顶部操作栏：刷新 + 保存 */
export function DetailHeader({ loading, saving, onRefresh, onSave }: { loading: boolean; saving: boolean; onRefresh: () => void; onSave: () => void }) {
  return (
    <div className="flex items-center justify-end gap-2 mb-4">
      <Button
        variant="flat"
        size="sm"
        isLoading={loading}
        onPress={onRefresh}
        startContent={<RefreshCw className="w-4 h-4" />}
      >
        刷新
      </Button>
      <Button
        color="primary"
        size="sm"
        isLoading={saving}
        onPress={onSave}
        startContent={<Save className="w-4 h-4" />}
      >
        保存配置
      </Button>
    </div>
  );
}

/** 双列表单栅格容器 */
export function FormGrid({ children }: { children: ReactNode }) {
  return <div className="grid grid-cols-1 md:grid-cols-2 gap-4">{children}</div>;
}

/** 详情页内分区：标题 + 描述 + 操作 + 主体内容 */
export function SectionBlock({ title, description, actions, children }: { title: string; description?: string; actions?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-small font-semibold text-foreground">{title}</h3>
          {description && <p className="text-tiny text-default-400 mt-0.5">{description}</p>}
        </div>
        {actions}
      </div>
      {children}
    </div>
  );
}

/** 详情页加载失败提示卡片（与列表页 ErrorState 对齐） */
export function DetailErrorState({ error }: { error: string }) {
  return (
    <Card className="border border-danger/30 bg-danger/10">
      <CardBody className="p-4">
        <p className="text-small font-semibold text-danger">数据加载失败</p>
        <p className="text-tiny text-danger mt-1 opacity-80">{error}</p>
      </CardBody>
    </Card>
  );
}

/** 详情页加载中占位 */
export function DetailLoading({ minHeight = 300 }: { minHeight?: number }) {
  return (
    <div className="flex items-center justify-center" style={{ minHeight }}>
      <Spinner size="lg" color="primary" />
    </div>
  );
}

/** 通用错误状态卡片：含重试按钮，用于列表页和详情页 */
export function ErrorState({ error, retry }: { error: string; retry: () => void }) {
  return (
    <Card className="border border-danger/30 bg-danger/10">
      <CardBody className="flex flex-row items-center justify-between gap-4 p-4">
        <div className="min-w-0">
          <p className="text-small font-semibold text-danger">数据加载失败</p>
          <p className="text-tiny text-danger mt-1 opacity-80 truncate">{error}</p>
        </div>
        <Button size="sm" color="danger" variant="flat" onPress={retry}>重试</Button>
      </CardBody>
    </Card>
  );
}

/** 通用加载中占位 */
export function LoadingState({ label = '加载中...', minHeight = 200 }: { label?: string; minHeight?: number }) {
  return (
    <div className="flex items-center justify-center" style={{ minHeight }}>
      <Spinner color="primary" label={label} />
    </div>
  );
}

/** 刷新态 hook：统一管理 silent 刷新时的视觉反馈（内容区 opacity-50 过渡） */
export function useRefreshState() {
  const [isRefreshing, setIsRefreshing] = useState(false);
  const wrap = useCallback(async <T,>(fn: () => Promise<T>, silent = false): Promise<T> => {
    if (silent) setIsRefreshing(true);
    try {
      return await fn();
    } finally {
      if (silent) setIsRefreshing(false);
    }
  }, []);
  return { isRefreshing, setIsRefreshing, wrap };
}

/** 统一页面标题栏：h2 标题 + text-tiny 副标题 + 可选状态 Chip + 右侧操作槽。
 * 所有自定义页面应使用此组件，确保 header 高度/字号/布局一致。 */
export function PageHeader({
  title,
  subtitle,
  icon: Icon,
  statusChip,
  actions,
}: {
  title: string;
  subtitle?: string;
  icon?: LucideIcon;
  statusChip?: { label: string; color?: 'success' | 'warning' | 'danger' | 'primary' | 'default'; pulse?: boolean };
  actions?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-divider">
      <div className="min-w-0">
        <div className="flex items-center gap-2 mb-1">
          {Icon && <Icon className="w-4 h-4 text-primary shrink-0" />}
          <h2 className="text-base font-bold text-foreground truncate">{title}</h2>
          {statusChip && (
            <Chip
              color={statusChip.color ?? 'default'}
              size="sm"
              variant="flat"
              startContent={statusChip.pulse ? <span className="w-2 h-2 rounded-full bg-current animate-pulse" /> : undefined}
            >
              {statusChip.label}
            </Chip>
          )}
        </div>
        {subtitle && <p className="text-tiny text-default-500">{subtitle}</p>}
      </div>
      {actions && <div className="flex items-center gap-2 shrink-0">{actions}</div>}
    </div>
  );
}

/** 统一刷新按钮：variant=flat, size=sm, RefreshCw w-4 h-4，文案默认"刷新" */
export function RefreshButton({ isLoading, onPress, label = '刷新' }: { isLoading?: boolean; onPress: () => void; label?: string }) {
  return (
    <Button variant="flat" size="sm" isLoading={isLoading} onPress={onPress} startContent={<RefreshCw className="w-4 h-4" />}>
      {label}
    </Button>
  );
}

/** 统一新建按钮：color=primary, size=sm, Plus w-4 h-4 */
export function CreateButton({ onPress, label = '新建', isLoading }: { onPress: () => void; label?: string; isLoading?: boolean }) {
  return (
    <Button color="primary" size="sm" isLoading={isLoading} onPress={onPress} startContent={<Plus className="w-4 h-4" />}>
      {label}
    </Button>
  );
}

/** 统一空状态：图标 + 标题 + 说明，参考 active-calls 实现。
 * 替代 HeroUI Table 的纯文字 emptyContent，提供一致的空状态体验。 */
export function EmptyState({
  icon: Icon,
  title,
  description,
}: {
  icon: LucideIcon;
  title: string;
  description?: string;
}) {
  return (
    <div className="flex flex-col items-center justify-center p-8 gap-3">
      <Icon className="w-8 h-8 text-default-400" aria-hidden />
      <div className="text-center">
        <p className="text-sm font-semibold text-foreground">{title}</p>
        {description && <p className="text-xs text-default-400 mt-1 max-w-sm">{description}</p>}
      </div>
    </div>
  );
}
