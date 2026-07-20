import { type ReactNode } from 'react';
import { Button, Card, CardBody, Spinner } from '@heroui/react';
import { RefreshCw, Save } from 'lucide-react';

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
