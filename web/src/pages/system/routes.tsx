// 系统管理 - 路由策略编排
// 主视图: 表格定义路由规则 (含时间路由字段)
// 每条规则支持独立的可视化拓扑编排 (点击"拓扑编排"按钮弹出画布)
// 画布节点配置与表格字段双向绑定

import { useMemo, useState } from 'react';
import {
  Button, Chip, Input, Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
  Card, CardBody,
} from '@heroui/react';
import { Send, GitFork, Clock, Network } from 'lucide-react';
import { api } from '@/services/client';
import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import type { ResourceSpec } from '@/pages/shared/types';
import type { Entity } from '@/services/resources';
import { message } from '@/utils/toast';
import { RouteTopologyEditor, type RouteRuleFields } from '@/components/ivr/route-rule-binding';

export function RoutesPage() {
  // 当前正在编辑拓扑的规则 (null 表示未打开)
  const [topoRule, setTopoRule] = useState<RouteRuleFields | null>(null);
  // 拓扑应用后的字段变更 (暂存,等用户在表格中保存)
  const [pendingChanges, setPendingChanges] = useState<Partial<RouteRuleFields>>({});

  // 路由规则 table spec - 含完整时间路由字段
  const routeSpec: ResourceSpec = useMemo(() => ({
    title: '路由规则',
    description: '按优先级匹配、改写号码并选择中继；支持时间段路由与权重分流。每条规则可独立编排可视化拓扑。',
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
      // 时间路由字段
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
    // 每行自定义操作: 拓扑编排按钮
    customRowAction: {
      label: '拓扑编排',
      color: 'secondary',
      onPress: (row: Entity) => {
        const rule: RouteRuleFields = {
          id: String(row.id ?? ''),
          prefix: row.prefix ? String(row.prefix) : undefined,
          priority: row.priority ? Number(row.priority) : undefined,
          gateway_id: row.gateway_id ? String(row.gateway_id) : undefined,
          cost: row.cost !== undefined ? Number(row.cost) : undefined,
          weight: row.weight ? Number(row.weight) : undefined,
          time_start: row.time_start ? String(row.time_start) : undefined,
          time_end: row.time_end ? String(row.time_end) : undefined,
          weekdays: row.weekdays ? String(row.weekdays) : undefined,
          timezone: row.timezone ? String(row.timezone) : undefined,
          caller_pattern: row.caller_pattern ? String(row.caller_pattern) : undefined,
          failover_strategy: row.failover_strategy ? String(row.failover_strategy) : undefined,
        };
        setPendingChanges({});
        setTopoRule(rule);
      },
    },
  }), []);

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

  const handleApplyTopology = (changes: Partial<RouteRuleFields>) => {
    setPendingChanges(changes);
    message.success('拓扑配置已应用,请在表格中点击"编辑"按钮保存到后端');
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
              <Chip size="sm" color="secondary" variant="flat">表格驱动 · 拓扑可视化</Chip>
            </div>
            <p className="text-xs text-default-500 mt-0.5">
              表格定义路由规则明细,每条规则支持独立的可视化拓扑编排,内置时间路由与主叫过滤
            </p>
          </div>
        </div>

        <Button
          color="primary"
          className="font-bold"
          startContent={<Send className="w-3.5 h-3.5" />}
          onPress={() => setSimOpen(true)}
        >
          路由仿真
        </Button>
      </div>

      {/* 主视图: 表格 (基于 ResourceWorkspace) */}
      <ResourceWorkspace spec={routeSpec} />

      {/* 时间路由使用说明 */}
      <Card className="shadow-sm">
        <CardBody className="p-4 flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <Clock className="w-4 h-4 text-amber-500" />
            <span className="text-sm font-bold">时间路由与拓扑编排使用说明</span>
          </div>
          <div className="text-xs text-default-600 dark:text-default-400 grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-1 pl-6">
            <p>· <span className="font-semibold">表格字段</span>: 直接编辑表格行即可修改规则参数</p>
            <p>· <span className="font-semibold">拓扑编排</span>: 点击每行"拓扑编排"按钮,可视化编辑该规则的处理流程</p>
            <p>· <span className="font-semibold">时间路由</span>: 填写 time_start/time_end/weekdays 即可启用</p>
            <p>· <span className="font-semibold">双向同步</span>: 画布节点配置修改后,点击"应用拓扑到表格"回写</p>
            <p>· <span className="font-semibold">工作时间</span>: 09:00-18:00 周一到周五 → 转坐席</p>
            <p>· <span className="font-semibold">非工作时间</span>: 转 IVR 自助服务或语音留言</p>
          </div>
        </CardBody>
      </Card>

      {/* 路由规则拓扑编排 Modal (每条规则独立画布) */}
      <Modal
        isOpen={topoRule !== null}
        onOpenChange={(o) => !o && setTopoRule(null)}
        size="full"
        scrollBehavior="outside"
      >
        <ModalContent>
          {() => (
            <>
              <ModalHeader className="flex items-center gap-2 border-b border-default-200 dark:border-slate-800">
                <Network className="w-5 h-5 text-purple-600" />
                <span>路由规则拓扑编排</span>
                {topoRule && (
                  <Chip size="sm" variant="flat" color="secondary" className="ml-2">
                    规则: {topoRule.id}
                  </Chip>
                )}
                {Object.keys(pendingChanges).length > 0 && (
                  <Chip size="sm" variant="flat" color="warning" className="ml-2">
                    有 {Object.keys(pendingChanges).length} 项变更待保存
                  </Chip>
                )}
              </ModalHeader>
              <ModalBody className="p-4">
                {topoRule && (
                  <RouteTopologyEditor
                    rule={topoRule}
                    onChange={handleApplyTopology}
                  />
                )}
                {Object.keys(pendingChanges).length > 0 && (
                  <div className="mt-2 p-3 bg-warning-50 dark:bg-warning-950/20 rounded-lg border border-warning-200 dark:border-warning-800">
                    <p className="text-xs font-bold text-warning-700 dark:text-warning-300 mb-2">
                      待应用的字段变更 (需在表格中编辑该规则并保存才能生效):
                    </p>
                    <div className="flex flex-wrap gap-2">
                      {Object.entries(pendingChanges).map(([k, v]) => (
                        <Chip key={k} size="sm" variant="flat">
                          <span className="font-mono">{k}</span>: {String(v ?? '(空)')}
                        </Chip>
                      ))}
                    </div>
                  </div>
                )}
              </ModalBody>
              <ModalFooter>
                <Button variant="flat" onPress={() => setTopoRule(null)}>
                  关闭
                </Button>
              </ModalFooter>
            </>
          )}
        </ModalContent>
      </Modal>

      {/* 路由仿真 Modal */}
      <Modal isOpen={simOpen} onOpenChange={(o) => !o && setSimOpen(false)} size="lg">
        <ModalContent>
          <ModalHeader>路由仿真测试</ModalHeader>
          <ModalBody>
            <div className="flex flex-col gap-2 py-2">
              <Input
                variant="bordered"
                label="目标号码"
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
