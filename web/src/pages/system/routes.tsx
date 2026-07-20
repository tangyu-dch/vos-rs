// 系统管理 - 路由策略与仿真
// 从 console.tsx 拆分

import { useMemo, useState } from 'react';
import {
  Button, Chip, Input, Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
} from '@heroui/react';
import { Send } from 'lucide-react';
import { api } from '@/services/client';
import { ResourceWorkspace, FieldLabel } from '@/pages/shared/resource-workspace';
import type { ResourceSpec } from '@/pages/shared/types';
import type { Entity } from '@/services/resources';

export function RoutesPage() {
  const routeSpec: ResourceSpec = useMemo(() => ({
    title: '路由策略', description: '按优先级匹配、改写号码并选择中继。', path: '/routing/rules',
    idKey: 'id', createLabel: '新建规则',
    fields: [
      { key: 'id', label: '规则 ID', required: true, placeholder: '例如 cn-mobile-primary' },
      { key: 'prefix', label: '匹配前缀', placeholder: '留空表示匹配全部号码' },
      { key: 'priority', label: '优先级', kind: 'number', required: true, defaultValue: 100 },
      { key: 'gateway_id', label: '目标中继', required: true, placeholder: '填写已存在的中继 ID' },
      { key: 'cost', label: '路由成本', kind: 'number', required: true, defaultValue: 0 },
      { key: 'weight', label: '分流权重', kind: 'number', min: 1, defaultValue: 100 },
      { key: 'time_start', label: '生效开始', placeholder: 'HH:MM，例如 08:00', pattern: /^([01]\d|2[0-3]):[0-5]\d$/, patternMessage: '请输入 HH:MM 格式的时间' },
      { key: 'time_end', label: '生效结束', placeholder: 'HH:MM，例如 22:00', pattern: /^([01]\d|2[0-3]):[0-5]\d$/, patternMessage: '请输入 HH:MM 格式的时间' },
    ],
  }), []);

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

  return (
    <div className="relative">
      <ResourceWorkspace spec={routeSpec} />
      <Button
        color="primary"
        className="fixed bottom-6 right-6 shadow-lg"
        startContent={<Send className="w-4 h-4" />}
        onPress={() => setSimOpen(true)}
      >
        路由仿真
      </Button>

      <Modal isOpen={simOpen} onOpenChange={(o) => !o && setSimOpen(false)} size="lg">
        <ModalContent>
          <ModalHeader>路由仿真</ModalHeader>
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
              <div className="mt-2 flex flex-col gap-3 p-4 rounded-2xl bg-slate-50 border border-slate-200/80">
                <div className="flex items-center justify-between">
                  <h4 className="text-xs font-bold text-slate-800 flex items-center gap-1.5">
                    <span className="w-2 h-2 rounded-full bg-emerald-500" />
                    匹配节点拓扑链 (Route Topology Graph)
                  </h4>
                  <Chip size="sm" color="success" variant="flat">匹配成功</Chip>
                </div>

                {/* 节点拓扑链 */}
                <div className="flex flex-wrap items-center gap-2 py-2 px-3 bg-white rounded-xl border border-slate-200/60 shadow-xs">
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-slate-400">呼入源</span>
                    <Chip size="sm" variant="bordered" className="font-semibold text-slate-700">INBOUND</Chip>
                  </div>
                  <span className="text-slate-300 font-bold">→</span>
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-slate-400">前缀匹配</span>
                    <Chip size="sm" color="primary" variant="flat" className="font-bold">
                      {String(simResult.prefix || '全前缀 *')}
                    </Chip>
                  </div>
                  <span className="text-slate-300 font-bold">→</span>
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-slate-400">优先级/成本</span>
                    <Chip size="sm" color="warning" variant="flat" className="font-semibold">
                      P:{String(simResult.priority ?? 100)} / C:{String(simResult.cost ?? 0)}
                    </Chip>
                  </div>
                  <span className="text-slate-300 font-bold">→</span>
                  <div className="flex flex-col items-center">
                    <span className="text-[10px] text-slate-400">落地网关</span>
                    <Chip size="sm" color="secondary" className="font-extrabold text-white">
                      {String(simResult.gateway_id || simResult.target_gateway || 'TRUNK-GW')}
                    </Chip>
                  </div>
                </div>

                <pre className="text-[11px] font-mono whitespace-pre-wrap text-slate-600 bg-slate-100 p-2.5 rounded-xl border border-slate-200/60 max-h-48 overflow-y-auto">{JSON.stringify(simResult, null, 2)}</pre>
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
