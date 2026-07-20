// 系统管理 - 路由策略与仿真
// 从 console.tsx 拆分

import { useMemo, useState } from 'react';
import {
  Button, Input, Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
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
              <div className="mt-2 p-3 rounded-medium bg-content2">
                <h4 className="text-small font-semibold text-foreground mb-2">匹配结果</h4>
                <pre className="text-tiny font-mono whitespace-pre-wrap text-default-600">{JSON.stringify(simResult, null, 2)}</pre>
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
