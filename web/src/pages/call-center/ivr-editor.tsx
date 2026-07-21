import { useCallback, useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { Button, Chip, Input } from '@heroui/react';
import { ArrowLeft, Save, Play, GitFork } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState, LoadingState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import { IvrCanvas, NodeInspector, NodePalette } from '@/components/ivr/ivr-canvas';
import {
  NODE_CATALOG_MAP, genNodeId,
  type IvrFlow, type IvrNode,
} from '@/components/ivr/types';

export default function IvrEditorPage() {
  const { id = '' } = useParams();
  const navigate = useNavigate();
  const [flow, setFlow] = useState<IvrFlow | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [ivrName, setIvrName] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const res: any = await api.get(`/ivr/menus/${id}`);
      const menu = res.data ?? res;
      const nodes: IvrNode[] = Array.isArray(menu.nodes) && menu.nodes.length > 0
        ? menu.nodes
        : [
            {
              id: genNodeId('start'),
              type: 'start',
              title: '呼入入口',
              description: `匹配 DID ${menu.did ?? '未指定'}`,
              position: { x: 80, y: 240 },
              config: { did: menu.did ?? '', welcome_prompt: menu.welcome_prompt ?? 'welcome.wav' },
            },
            {
              id: genNodeId('menu'),
              type: 'menu',
              title: '多级菜单',
              description: '主菜单分支',
              position: { x: 380, y: 240 },
              config: { ...NODE_CATALOG_MAP.menu.defaultConfig },
            },
            {
              id: genNodeId('hangup'),
              type: 'hangup',
              title: '挂断',
              description: '结束通话',
              position: { x: 700, y: 240 },
              config: { reason: 'normal', playbye: true },
            },
          ];
      setFlow({
        id: menu.id,
        name: menu.name,
        description: menu.description,
        did: menu.did,
        welcome_prompt: menu.welcome_prompt,
        timeout_secs: menu.timeout_secs ?? 30,
        enabled: menu.enabled ?? true,
        nodes,
        edges: Array.isArray(menu.edges) ? menu.edges : [],
      });
      setIvrName(menu.name ?? '');
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载 IVR 流程失败');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => { void load(); }, [load]);

  const handleSave = async () => {
    if (!flow) return;
    setSaving(true);
    try {
      await api.put(`/ivr/menus/${flow.id}`, {
        ...flow,
        name: ivrName,
        nodes: flow.nodes,
        edges: flow.edges,
      });
      message.success('IVR 流程已保存');
    } catch (err) {
      message.error(err instanceof Error ? err.message : '保存失败');
    } finally {
      setSaving(false);
    }
  };

  const selectedNode = flow?.nodes.find((n) => n.id === selectedNodeId) ?? null;

  const handleNodeChange = (updated: IvrNode) => {
    if (!flow) return;
    setFlow({
      ...flow,
      nodes: flow.nodes.map((n) => (n.id === updated.id ? updated : n)),
    });
  };

  if (loading) {
    return (
      <div className="p-6">
        <LoadingState />
      </div>
    );
  }

  if (error) {
    return (
      <div className="p-6">
        <ErrorState error={error} retry={load} />
      </div>
    );
  }

  if (!flow) return null;

  return (
    <div className="flex flex-col gap-3 h-full">
      {/* 顶部工具栏 */}
      <div className="flex items-center justify-between gap-4 p-4 bg-content1 rounded-xl border border-default-200 dark:border-slate-800">
        <div className="flex items-center gap-3">
          <Button
            isIconOnly
            size="sm"
            variant="flat"
            onPress={() => navigate('/ivr')}
          >
            <ArrowLeft className="w-4 h-4" />
          </Button>
          <GitFork className="w-5 h-5 text-purple-600" />
          <Input
            variant="underlined"
            className="max-w-xs"
            value={ivrName}
            onValueChange={setIvrName}
            classNames={{ input: 'text-base font-bold' }}
          />
          <Chip size="sm" variant="flat" color="secondary">
            {flow.nodes.length} 节点 · {flow.edges.length} 连线
          </Chip>
          {flow.did && <Chip size="sm" variant="flat" color="primary">DID {flow.did}</Chip>}
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="flat"
            startContent={<Play className="w-3.5 h-3.5" />}
            onPress={() => message.info('仿真调试功能即将上线')}
          >
            仿真调试
          </Button>
          <Button
            size="sm"
            color="secondary"
            className="font-bold text-white"
            startContent={<Save className="w-3.5 h-3.5" />}
            onPress={handleSave}
            isLoading={saving}
          >
            保存流程
          </Button>
        </div>
      </div>

      {/* 三栏布局: 左侧 palette + 中间 canvas + 右侧 inspector */}
      <div className="flex gap-3 flex-1 min-h-0">
        <NodePalette />
        <div className="flex-1 min-w-0">
          <IvrCanvas
            flow={flow}
            onChange={setFlow}
            selectedNodeId={selectedNodeId}
            onSelectNode={setSelectedNodeId}
          />
        </div>
        <NodeInspector node={selectedNode} onChange={handleNodeChange} />
      </div>
    </div>
  );
}
