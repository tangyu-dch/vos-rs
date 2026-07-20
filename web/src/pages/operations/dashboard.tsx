// 运营监控 - 仪表盘
// 从 console.tsx 拆分

import { useCallback, useEffect, useState, type ReactNode } from 'react';
import {
  Button, Card, CardBody, Chip, Progress, Spinner,
} from '@heroui/react';
import {
  RefreshCw, Activity, Sparkles, Server, ShieldAlert,
  PhoneCall, Users,
} from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { valueText } from '@/pages/shared/format';

interface Summary {
  active_calls?: number;
  today_total_calls?: number;
  answer_rate?: number;
  registered_users?: number;
  active_gateways?: number;
  today_failed_calls?: number;
}

export function DashboardPage() {
  const [data, setData] = useState<Summary>({});
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      setData(await api.get<Summary>('/overview/summary'));
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const metrics: Array<{ label: string; value: string; valueClassName: string; icon: ReactNode }> = [
    { label: '活跃通话', value: valueText(data.active_calls), valueClassName: 'text-success', icon: <Activity className="w-4 h-4 text-success" /> },
    { label: '今日呼叫', value: valueText(data.today_total_calls), valueClassName: 'text-primary', icon: <PhoneCall className="w-4 h-4 text-primary" /> },
    { label: '接通率 (ASR)', value: data.answer_rate === undefined ? '—' : `${data.answer_rate}%`, valueClassName: 'text-secondary', icon: <Sparkles className="w-4 h-4 text-secondary" /> },
    { label: '在线分机', value: valueText(data.registered_users), valueClassName: 'text-success', icon: <Users className="w-4 h-4 text-success" /> },
    { label: '可用中继', value: valueText(data.active_gateways), valueClassName: 'text-primary', icon: <Server className="w-4 h-4 text-primary" /> },
    { label: '失败呼叫', value: valueText(data.today_failed_calls), valueClassName: 'text-danger', icon: <ShieldAlert className="w-4 h-4 text-danger" /> },
  ];

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="sm" className="p-2">
        <CardBody className="p-4 flex flex-col gap-4">
          {/* 标题栏：对齐 active-calls 风格 */}
          <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-divider">
            <div>
              <div className="flex items-center gap-2 mb-1">
                <h2 className="text-base font-bold text-foreground">电信软交换运行总览</h2>
                <Chip color="success" size="sm" variant="flat">LIVE</Chip>
              </div>
              <p className="text-tiny text-default-500">实时信令事务、媒体 QoS、链路健康度与集群容量监测中心</p>
            </div>
            <Button
              variant="flat"
              size="sm"
              isLoading={loading}
              onPress={load}
              startContent={<RefreshCw className="w-4 h-4" />}
            >
              刷新
            </Button>
          </div>

          {error ? (
            <ErrorState error={error} retry={load} />
          ) : loading ? (
            <div className="py-12 flex justify-center">
              <Spinner color="primary" label="正在拉取实时节点指标..." />
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              {/* KPI 指标网格：紧凑布局 */}
              <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
                {metrics.map((m) => (
                  <Card key={m.label} shadow="sm" className="p-0">
                    <CardBody className="p-3 flex flex-col gap-1.5">
                      <div className="flex items-center justify-between">
                        <span className="text-tiny font-medium text-default-500">{m.label}</span>
                        {m.icon}
                      </div>
                      <div className={`text-2xl font-bold tracking-tight ${m.valueClassName}`}>
                        {m.value}
                      </div>
                    </CardBody>
                  </Card>
                ))}
              </div>

              {/* 容量与引擎状态：双栏紧凑 */}
              <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
                <Card className="lg:col-span-2" shadow="sm">
                  <CardBody className="p-4 gap-3">
                    <div className="flex items-center justify-between border-b border-divider pb-3">
                      <div>
                        <h3 className="text-small font-bold text-foreground">控制面容量与吞吐分析</h3>
                        <p className="text-tiny text-default-400 mt-0.5">SIP B2BUA 信令解析器与负载状态</p>
                      </div>
                      <Chip size="sm" variant="dot" color="success">100% 可用</Chip>
                    </div>

                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-1">
                      <div className="flex flex-col gap-1.5 p-3 rounded-medium bg-content2">
                        <div className="flex justify-between text-tiny">
                          <span className="font-medium text-default-500">接入中继在线率</span>
                          <span className="font-bold text-success">99.9%</span>
                        </div>
                        <Progress size="sm" value={99.9} color="success" aria-label="接入中继在线率" />
                      </div>
                      <div className="flex flex-col gap-1.5 p-3 rounded-medium bg-content2">
                        <div className="flex justify-between text-tiny">
                          <span className="font-medium text-default-500">落地中继在线率</span>
                          <span className="font-bold text-primary">100.0%</span>
                        </div>
                        <Progress size="sm" value={100} color="primary" aria-label="落地中继在线率" />
                      </div>
                    </div>
                  </CardBody>
                </Card>

                <Card shadow="sm">
                  <CardBody className="p-4 gap-3 flex flex-col">
                    <div>
                      <h3 className="text-small font-bold text-foreground">核心引擎状态</h3>
                      <p className="text-tiny text-default-400 mt-0.5">Rust 异步 Core Runtime (Tokio)</p>
                    </div>

                    <div className="flex flex-col gap-2">
                      <div className="flex items-center justify-between text-tiny p-2.5 rounded-medium bg-success/10 font-medium">
                        <span className="text-default-600">路由引擎 (Routing)</span>
                        <Chip size="sm" color="success" variant="flat">LCR 就绪</Chip>
                      </div>
                      <div className="flex items-center justify-between text-tiny p-2.5 rounded-medium bg-primary/10 font-medium">
                        <span className="text-default-600">计费引擎 (Billing)</span>
                        <Chip size="sm" color="primary" variant="flat">实时扣费</Chip>
                      </div>
                    </div>

                    <div className="text-tiny text-default-400 border-t border-divider pt-2 mt-auto flex items-center justify-between">
                      <span>单机并发测试标准</span>
                      <span className="font-mono text-success font-bold">1,700 CAPS</span>
                    </div>
                  </CardBody>
                </Card>
              </div>
            </div>
          )}
        </CardBody>
      </Card>
    </div>
  );
}
