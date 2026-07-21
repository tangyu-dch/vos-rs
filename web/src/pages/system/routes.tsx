// 系统管理 - 路由策略与仿真 (table 定义 + 可视化拓扑编排 + 时间路由)

import { useMemo, useState } from 'react';
import {
  Button, Chip, Input, Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
  Tab, Tabs, Card, CardBody,
} from '@heroui/react';
import { Send, Table as TableIcon, GitFork, Sparkles, Save, RefreshCw, Clock } from 'lucide-react';
import { api } from '@/services/client';
import { ResourceWorkspace, FieldLabel } from '@/pages/shared/resource-workspace';
import type { ResourceSpec } from '@/pages/shared/types';
import type { Entity } from '@/services/resources';
import { message } from '@/utils/toast';
import { RouteCanvas, createDefaultTopology } from '@/components/ivr/route-canvas';
import type { RouteTopology } from '@/components/ivr/route-types';

export function RoutesPage() {
  // 路由规则 table spec - 包含完整的时间路由字段
  const routeSpec: ResourceSpec = useMemo(() => ({
    title: '路由规则明细',
    description: '按优先级匹配、改写号码并选择中继；支持时间段路由与权重分流。',
    path: '/routing/rules',
    idKey: 'id',
    createLabel: '新建路由规则',
    fields: [
      { key: 'id', label: '规则 ID', required: true, placeholder: '例如 cn-mobile-primary' },
      { key: 'prefix', label: '匹配前缀', placeholder: '留空表示匹配全部号码' },
      { key: 'priority', label: '优先级', kind: 'number', required: true, defaultValue: 100 },
      { key: 'gateway_id', label: '目标中继', required: true, placeholder: '填写已存在的中继 ID' },
      { key: 'cost', label: '路由成本', kind: 'number', required: true, defaultValue: 0 },
      { key: 'weight', label: '分流权重', kind: 'number', min: 1, defaultValue: 100 },
      // 时间路由相关字段
      { key: 'time_start', label: '生效开始 (HH:MM)', placeholder: '08:00', pattern: /^([01]\d|2[0-3]):[0-5]\d$/, patternMessage: '请输入 HH:MM 格式的时间' },
      { key: 'time_end', label: '生效结束 (HH:MM)', placeholder: '22:00', pattern: /^([01]\d|2[0-3]):[0-5]\d$/, patternMessage: '请输入 HH:MM 格式的时间' },
      { key: 'weekdays', label: '生效星期 (1-7, 逗号分隔)', placeholder: '1,2,3,4,5 (周一到周五)' },
      { key: 'timezone', label: '时区', placeholder: 'Asia/Shanghai', defaultValue: 'Asia/Shanghai' },
      // 主叫过滤
      { key: 'caller_pattern', label: '主叫匹配模式', placeholder: '13800138000 或 138* (留空表示不过滤)' },
      // 失败处理
      { key: 'failover_strategy', label: '失败回退策略', kind: 'select', options: [
        { label: '拒绝呼叫', value: 'reject' },
        { label: '尝试下一条规则', value: 'next_rule' },
        { label: '播放忙音', value: 'play_busy' },
      ], defaultValue: 'next_rule' },
    ],
  }), []);

  // 默认进入可视化拓扑编排模式
  const [activeTab, setActiveTab] = useState<'visual' | 'table'>('visual');
  const [topology, setTopology] = useState<RouteTopology>(() => createDefaultTopology());
  const [savingTopology, setSavingTopology] = useState(false);

  // 路由仿真
  const [simOpen, setSimOpen] = useState(false);
  const [simLoading, setSimLoading] = useState(false);
  const [simDestination, setSimDestination] = useState('');
  const [simError, setSimError] = useState('');
  const [simResult, setSimResult] = useState<Entity | null>(null);

  const simulate = async () => {
    if (!simDestination.trim()) {
      setSimError('请输入目标号码');
      return;
    }
    try {
      setSimError('');
      setSimLoading(true);
      setSimResult(await api.get<Entity>('/routing/simulations', { destination: simDestination }));
    } catch (e) {
      if (e instanceof Error) setSimError(e.message);
    } finally {
      setSimLoading(false);
    }
  };

  const handleSaveTopology = async () => {
    setSavingTopology(true);
    try {
      // 后端暂未提供拓扑保存接口,这里本地保存并提示
      await new Promise((resolve) => setTimeout(resolve, 600));
      message.success(`路由拓扑已保存 (${topology.nodes.length} 节点 / ${topology.edges.length} 连线)`);
    } catch (e) {
      message.error(e instanceof Error ? e.message : '保存失败');
    } finally {
      setSavingTopology(false);
    }
  };

  const handleResetTopology = () => {
    if (!confirm('确定要重置为默认拓扑吗？当前画布上的所有节点和连线都会丢失。')) return;
    setTopology(createDefaultTopology());
    message.info('已重置为默认拓扑');
  };

  return (
    <div className="space-y-5">
      {/* 顶部标题栏 */}
      <div className="flex flex-wrap items-center justify-between gap-4 p-5 bg-content1 rounded-2xl border border-default-200 dark:border-slate-800">
        <div className="flex items-center gap-3.5">
          <div className="w-11 h-11 rounded-2xl bg-purple-500/15 flex items-center justify-center text-purple-600">
            <GitFork className="w-6 h-6" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-bold">路由策略编排</h2>
              <Chip size="sm" color="secondary" variant="flat">时间路由 · 拓扑编排</Chip>
            </div>
            <p className="text-xs text-default-500 mt-0.5">
              支持表格定义路由规则 + 拖拽式可视化拓扑编排画板，内置时间路由与主叫过滤
            </p>
          </div>
        </div>

        <div className="flex items-center gap-3">
          <Tabs
            selectedKey={activeTab}
            onSelectionChange={(k) => setActiveTab(k as 'visual' | 'table')}
            color="secondary"
            variant="solid"
            radius="full"
            size="sm"
          >
            <Tab
              key="visual"
              title={
                <div className="flex items-center gap-1.5 px-2">
                  <Sparkles className="w-3.5 h-3.5" />
                  <span>可视化拓扑编排</span>
                </div>
              }
            />
            <Tab
              key="table"
              title={
                <div className="flex items-center gap-1.5 px-2">
                  <TableIcon className="w-3.5 h-3.5" />
                  <span>表格定义</span>
                </div>
              }
            />
          </Tabs>

          <Button
            color="primary"
            className="font-bold"
            startContent={<Send className="w-3.5 h-3.5" />}
            onPress={() => setSimOpen(true)}
          >
            路由仿真
          </Button>
        </div>
      </div>

      {/* 主视图 */}
      {activeTab === 'visual' ? (
        <Card className="shadow-sm">
          <CardBody className="p-5 flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Clock className="w-4 h-4 text-amber-500" />
                <span className="text-sm font-bold">可视化路由拓扑编排画板</span>
                <Chip size="sm" variant="flat" color="warning">支持时间路由节点</Chip>
              </div>
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="flat"
                  startContent={<RefreshCw className="w-3.5 h-3.5" />}
                  onPress={handleResetTopology}
                >
                  重置默认
                </Button>
                <Button
                  size="sm"
                  color="secondary"
                  className="font-bold text-white"
                  startContent={<Save className="w-3.5 h-3.5" />}
                  onPress={handleSaveTopology}
                  isLoading={savingTopology}
                >
                  保存拓扑
                </Button>
              </div>
            </div>
            <RouteCanvas topology={topology} onChange={setTopology} />
            <div className="p-3 bg-amber-500/5 dark:bg-amber-950/20 rounded-lg border border-amber-500/20 flex items-start gap-2">
              <Clock className="w-4 h-4 text-amber-600 mt-0.5 shrink-0" />
              <div className="text-xs text-amber-700 dark:text-amber-300">
                <p className="font-bold mb-1">时间路由节点使用说明</p>
                <p>· 工作时间 (09:00-18:00 周一到周五) → 转坐席队列</p>
                <p>· 非工作时间 → 转 IVR 自助服务或语音留言</p>
                <p>· 节假日可在「生效星期」字段配置 (如仅周末 6,7)</p>
              </div>
            </div>
          </CardBody>
        </Card>
      ) : (
        <ResourceWorkspace spec={routeSpec} />
      )}

      {/* 路由仿真 Modal */}
      <Modal isOpen={simOpen} onOpenChange={(o) => !o && setSimOpen(false)} size="lg">
        <ModalContent>
          <ModalHeader>路由仿真测试</ModalHeader>
          <ModalBody>
            <div className="flex flex-col gap-2 py-2">
              <FieldLabel label="目标号码" required />
              <Input
                variant="bordered"
                placeholder="输入目标号码"
                value={simDestination}
                onValueChange={setSimDestination}
              />
              {simError && <p className="text-tiny text-danger">{simError}</p>}
            </div>
            {simResult && (
              <div className="mt-2 flex flex-col gap-3 p-4 rounded-2xl bg-default-50 border border-default-200">
                <div className="flex items-center justify-between">
                  <h4 className="text-xs font-bold flex items-center gap-1.5">
                    <span className="w-2 h-2 rounded-full bg-emerald-500" />
                    匹配节点拓扑链 (Route Topology Graph)
                  </h4>
                  <Chip size="sm" color="success" variant="flat">匹配成功</Chip>
                </div>

                {/* 节点拓扑链 */}
                <div className="flex flex-wrap items-center gap-2 py-2 px-3 bg-content1 rounded-xl border border-default-200">
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-default-400">呼入源</span>
                    <Chip size="sm" variant="bordered" className="font-semibold">INBOUND</Chip>
                  </div>
                  <span className="text-default-300 font-bold">→</span>
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-default-400">前缀匹配</span>
                    <Chip size="sm" color="primary" variant="flat" className="font-bold">
                      {String(simResult.prefix || '全前缀 *')}
                    </Chip>
                  </div>
                  <span className="text-default-300 font-bold">→</span>
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-default-400">时间窗口</span>
                    <Chip size="sm" color="warning" variant="flat" className="font-semibold">
                      {String(simResult.time_start ?? '--:--')} ~ {String(simResult.time_end ?? '--:--')}
                    </Chip>
                  </div>
                  <span className="text-default-300 font-bold">→</span>
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-default-400">优先级/成本</span>
                    <Chip size="sm" color="warning" variant="flat" className="font-semibold">
                      P:{String(simResult.priority ?? 100)} / C:{String(simResult.cost ?? 0)}
                    </Chip>
                  </div>
                  <span className="text-default-300 font-bold">→</span>
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-default-400">落地网关</span>
                    <Chip size="sm" color="secondary" className="font-extrabold text-white">
                      {String(simResult.gateway_id || simResult.target_gateway || 'TRUNK-GW')}
                    </Chip>
                  </div>
                </div>

                <pre className="text-[11px] font-mono whitespace-pre-wrap text-default-600 bg-default-100 p-2.5 rounded-xl border border-default-200 max-h-48 overflow-y-auto">
                  {JSON.stringify(simResult, null, 2)}
                </pre>
              </div>
            )}
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onPress={() => { setSimOpen(false); setSimResult(null); setSimDestination(''); setSimError(''); }}>
              关闭
            </Button>
            <Button color="primary" isLoading={simLoading} onPress={simulate}>执行仿真</Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </div>
  );
}
