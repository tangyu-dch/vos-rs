import { useState } from 'react';
import {
  Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
  Button, Chip, Card, CardBody
} from '@heroui/react';
import { Network, Plus, ArrowRight, Play, Clock, PhoneForwarded, Settings, CheckCircle2 } from 'lucide-react';

export interface FlowNode {
  id: string;
  type: 'start' | 'time_filter' | 'prefix_match' | 'ivr_prompt' | 'gateway_trunk';
  title: string;
  subtitle: string;
  config: Record<string, string>;
}

interface VisualFlowEditorProps {
  isOpen: boolean;
  onClose: () => void;
}

export function VisualFlowEditor({ isOpen, onClose }: VisualFlowEditorProps) {
  const [nodes, setNodes] = useState<FlowNode[]>([
    {
      id: 'node-1',
      type: 'start',
      title: '呼入入口 (Inbound Trigger)',
      subtitle: '匹配 DID 号码 400-800-9000',
      config: { did: '4008009000' }
    },
    {
      id: 'node-2',
      type: 'time_filter',
      title: '工作时间检查 (Time Window)',
      subtitle: '08:30 - 18:00 (工作日)',
      config: { start: '08:30', end: '18:00' }
    },
    {
      id: 'node-3',
      type: 'ivr_prompt',
      title: 'IVR 语音导航 (Main Menu)',
      subtitle: '按 1 转售前，按 2 转售后支持',
      config: { prompt: 'welcome_zh.wav' }
    },
    {
      id: 'node-4',
      type: 'gateway_trunk',
      title: '落地中继网关 (Trunk Target)',
      subtitle: 'PRIORITY: 10 | COST: 0.02',
      config: { gateway_id: 'gw-shanghai-primary' }
    }
  ]);

  const addNode = (type: FlowNode['type']) => {
    const newId = `node-${nodes.length + 1}`;
    let title = '新路由节点';
    let subtitle = '配置详细规则';

    if (type === 'prefix_match') {
      title = '前缀校验 (Prefix Match)';
      subtitle = '匹配号码前缀 86138*';
    } else if (type === 'gateway_trunk') {
      title = '备用中继 (Backup Gateway)';
      subtitle = 'PRIORITY: 50 | GW: gw-beijing-backup';
    }

    setNodes([...nodes, { id: newId, type, title, subtitle, config: {} }]);
  };

  const getIcon = (type: FlowNode['type']) => {
    switch (type) {
      case 'start': return <Play className="w-4 h-4 text-emerald-600" />;
      case 'time_filter': return <Clock className="w-4 h-4 text-amber-600" />;
      case 'ivr_prompt': return <Settings className="w-4 h-4 text-indigo-600" />;
      case 'gateway_trunk': return <PhoneForwarded className="w-4 h-4 text-blue-600" />;
      default: return <Network className="w-4 h-4 text-slate-600" />;
    }
  };

  return (
    <Modal isOpen={isOpen} onOpenChange={(o) => !o && onClose()} size="5xl">
      <ModalContent className="max-w-6xl">
        <ModalHeader className="flex items-center justify-between border-b border-slate-100 pb-3">
          <div className="flex items-center gap-2">
            <Network className="w-5 h-5 text-indigo-600" />
            <span className="text-base font-bold text-slate-800">
              Drag-and-Drop 可视化路由与 IVR 节点编排器
            </span>
          </div>
          <Chip color="success" size="sm" variant="flat" startContent={<CheckCircle2 className="w-3.5 h-3.5" />}>
            实时校验激活
          </Chip>
        </ModalHeader>
        <ModalBody className="py-6 bg-slate-50/50">
          <div className="flex flex-col gap-6">
            {/* 节点工具栏 */}
            <div className="flex items-center gap-2 bg-white p-3 rounded-xl border border-slate-200/80 shadow-xs">
              <span className="text-xs font-bold text-slate-700 mr-2">添加节点:</span>
              <Button size="sm" variant="flat" color="primary" startContent={<Plus className="w-3.5 h-3.5" />} onPress={() => addNode('prefix_match')}>
                + 前缀匹配节点
              </Button>
              <Button size="sm" variant="flat" color="secondary" startContent={<Plus className="w-3.5 h-3.5" />} onPress={() => addNode('gateway_trunk')}>
                + 中继落地节点
              </Button>
            </div>

            {/* 可视化拓扑节点链 */}
            <div className="flex flex-wrap items-center gap-4 py-8 px-6 bg-white rounded-2xl border border-slate-200 shadow-sm overflow-x-auto min-h-[220px]">
              {nodes.map((node, idx) => (
                <div key={node.id} className="flex items-center gap-4">
                  <Card className="w-56 border border-slate-200/90 hover:border-indigo-400 hover:shadow-md transition-all">
                    <CardBody className="p-3.5 flex flex-col gap-1.5">
                      <div className="flex items-center justify-between">
                        <div className="p-1.5 rounded-lg bg-slate-100">{getIcon(node.type)}</div>
                        <span className="text-[10px] font-mono text-slate-400">#{node.id}</span>
                      </div>
                      <span className="text-xs font-bold text-slate-800 mt-1">{node.title}</span>
                      <span className="text-[11px] text-slate-500 font-mono">{node.subtitle}</span>
                    </CardBody>
                  </Card>
                  {idx < nodes.length - 1 && (
                    <div className="flex flex-col items-center">
                      <ArrowRight className="w-5 h-5 text-indigo-500 stroke-[2.5]" />
                      <span className="text-[9px] font-bold text-indigo-400">PASS</span>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </div>
        </ModalBody>
        <ModalFooter className="border-t border-slate-100 pt-3">
          <Button variant="flat" onPress={onClose}>
            取消
          </Button>
          <Button color="primary" onPress={onClose}>
            保存路由编排图
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
