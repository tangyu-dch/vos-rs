import { useState } from 'react';
import {
  Button,
  Chip,
  Card,
  CardBody,
  Input,
  Modal,
  ModalContent,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from '@heroui/react';
import { Send, Route as RouteIcon, ArrowRight, ShieldCheck, Cpu } from 'lucide-react';
import { api } from '@/services/client';
import { ResourceWorkspace } from '@/components/resource-workspace';
import { sipRoutes } from '@/pages/shared/resource-specs';
import type { Entity } from '@/services/resources';

export function RoutesPage() {
  // 路由仿真
  const [simOpen, setSimOpen] = useState(false);
  const [simLoading, setSimLoading] = useState(false);
  const [simDestination, setSimDestination] = useState('');
  const [simAccessTrunkId, setSimAccessTrunkId] = useState('');
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
      const params: Record<string, string> = { destination: simDestination.trim() };
      if (simAccessTrunkId.trim()) {
        params.access_trunk_id = simAccessTrunkId.trim();
      }
      setSimResult(await api.get<Entity>('/routing/simulations', params));
    } catch (e) {
      if (e instanceof Error) setSimError(e.message);
    } finally {
      setSimLoading(false);
    }
  };

  return (
    <div className="space-y-4">
      {/* 传显 CRS 拓扑全景指引 Banner */}
      <Card className="bg-gradient-to-r from-slate-900/90 via-purple-950/40 to-blue-950/40 border border-purple-500/20 shadow-md">
        <CardBody className="p-4">
          <div className="flex flex-col md:flex-row items-start md:items-center justify-between gap-4">
            <div className="flex items-center gap-3">
              <div className="p-2.5 rounded-xl bg-purple-500/10 border border-purple-500/20 text-purple-400">
                <RouteIcon className="w-6 h-6" />
              </div>
              <div>
                <h3 className="text-base font-semibold text-slate-100 flex items-center gap-2">
                  CRS 呼出路由控制中心
                  <Chip size="sm" color="secondary" variant="flat">极简选路</Chip>
                </h3>
                <p className="text-xs text-slate-400 mt-0.5">
                  按开户租户与被叫前缀精准寻路，自动完成加头剪切改写，顺延并发选路。
                </p>
              </div>
            </div>

            {/* 可视化选路拓扑链 */}
            <div className="flex items-center gap-2 text-xs bg-slate-900/80 px-3.5 py-2 rounded-lg border border-slate-800">
              <div className="flex items-center gap-1.5 text-slate-300">
                <ShieldCheck className="w-3.5 h-3.5 text-cyan-400" />
                <span>开户租户/请求</span>
              </div>
              <ArrowRight className="w-3.5 h-3.5 text-slate-500" />
              <div className="flex items-center gap-1.5 text-purple-300 font-medium">
                <RouteIcon className="w-3.5 h-3.5 text-purple-400" />
                <span>前缀最长匹配</span>
              </div>
              <ArrowRight className="w-3.5 h-3.5 text-slate-500" />
              <div className="flex items-center gap-1.5 text-emerald-300">
                <Cpu className="w-3.5 h-3.5 text-emerald-400" />
                <span>落地物理中继</span>
              </div>
            </div>
          </div>
        </CardBody>
      </Card>

      {/* 主视图: 表格 (基于 ResourceWorkspace) */}
      <ResourceWorkspace
        spec={sipRoutes}
        headerActionsPermission="routing.simulate"
        headerActions={
          <Button
            color="primary"
            size="sm"
            className="font-bold text-white bg-primary hover:bg-primary/80"
            startContent={<Send className="w-3.5 h-3.5" />}
            onPress={() => setSimOpen(true)}
          >
            路由仿真
          </Button>
        }
      />

      {/* 路由仿真 Modal */}
      <Modal isOpen={simOpen} onOpenChange={(o) => !o && setSimOpen(false)} size="4xl">
        <ModalContent>
          <ModalHeader>路由仿真测试</ModalHeader>
          <ModalBody>
            <div className="flex flex-col gap-2 py-2">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <Input
                  variant="bordered"
                  label="目标号码 (被叫)"
                  placeholder="输入目标号码 (如 13800001001)"
                  value={simDestination}
                  onValueChange={setSimDestination}
                />
                <Input
                  variant="bordered"
                  label="接入中继 (模拟接入源)"
                  placeholder="请输入接入中继标识 (例如 access-alpha)"
                  value={simAccessTrunkId}
                  onValueChange={setSimAccessTrunkId}
                />
              </div>
              {simError && <p className="text-tiny text-danger">{simError}</p>}
            </div>
            {simResult && (
              <div className="mt-2 flex flex-col gap-3 p-4 rounded-2xl bg-content2 border border-default-200">
                <div className="flex items-center justify-between">
                  <h4 className="text-xs font-bold flex items-center gap-1.5">
                    <span className="w-2 h-2 rounded-full bg-success" />
                    匹配节点拓扑链 (Route Topology Graph)
                  </h4>
                  <Chip size="sm" color="success" variant="flat">
                    匹配成功
                  </Chip>
                </div>

                {/* 选号显号信息 */}
                {Boolean(simResult.selected_caller_number || simResult.caller_pool_id) && (
                  <div className="flex items-center gap-3 p-2.5 rounded-xl bg-primary-50 dark:bg-primary-900/20 border border-primary-200 dark:border-primary-800">
                    <span className="text-xs font-semibold text-primary">号码池抽号结果:</span>
                    <Chip size="sm" color="primary" variant="solid" className="font-mono font-bold">
                      主叫号码: {String(simResult.selected_caller_number || '无')}
                    </Chip>
                    {Boolean(simResult.caller_pool_id) && (
                      <Chip size="sm" color="secondary" variant="flat" className="font-mono">
                        号码池: {String(simResult.caller_pool_id)}
                      </Chip>
                    )}
                  </div>
                )}

                {/* 节点拓扑链 */}
                {(() => {
                  const candidates = Array.isArray(simResult.candidates) ? simResult.candidates : [];
                  const firstCandidate = candidates[0] || {};
                  const gatewayId = String(
                    firstCandidate.gateway_id || simResult.gateway_id || simResult.target_gateway || 'TRUNK-GW'
                  );
                  const host = firstCandidate.host ? `${firstCandidate.host}:${firstCandidate.port || 5060}` : '';
                  return (
                    <div className="flex flex-wrap items-center gap-2 py-2 px-3 bg-content1 rounded-xl border border-default-200">
                      <div className="flex flex-col items-center">
                        <span className="text-[10px] text-default-400">接入源/中继</span>
                        <Chip size="sm" variant="bordered" className="font-semibold">
                          {String(simResult.access_trunk_id || 'INBOUND')}
                        </Chip>
                      </div>
                      <span className="text-default-300 font-bold">→</span>
                      <div className="flex flex-col items-center">
                        <span className="text-[10px] text-default-400">前缀规则</span>
                        <Chip size="sm" color="primary" variant="flat" className="font-bold">
                          {String(firstCandidate.route_id || simResult.prefix || '前缀匹配')}
                        </Chip>
                      </div>
                      <span className="text-default-300 font-bold">→</span>
                      <div className="flex flex-col items-center">
                        <span className="text-[10px] text-default-400">落地网关</span>
                        <Chip size="sm" color="primary" className="font-extrabold text-white">
                          {gatewayId} {host ? `(${host})` : ''}
                        </Chip>
                      </div>
                      {candidates.length > 1 && (
                        <Chip size="sm" color="warning" variant="flat" className="text-[10px] ml-auto">
                          包含 {candidates.length} 个 Failover 备用节点
                        </Chip>
                      )}
                    </div>
                  );
                })()}

                <pre className="text-[11px] font-mono whitespace-pre-wrap text-default-600 bg-default-100 p-2.5 rounded-xl border border-default-200 max-h-48 overflow-y-auto">
                  {JSON.stringify(simResult, null, 2)}
                </pre>
              </div>
            )}
          </ModalBody>
          <ModalFooter>
            <Button
              variant="flat"
              onPress={() => {
                setSimOpen(false);
                setSimResult(null);
                setSimDestination('');
                setSimAccessTrunkId('');
                setSimError('');
              }}
            >
              关闭
            </Button>
            <Button color="primary" isLoading={simLoading} onPress={simulate}>
              执行仿真
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </div>
  );
}
