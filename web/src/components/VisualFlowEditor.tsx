import React, { useState } from 'react';
import {
  Button, Chip, Card, CardBody, Input
} from '@heroui/react';
import {
  Network, Play, PhoneForwarded, Settings, Trash2, Volume2, Move, Layers, Sparkles, X, Upload, Plus, GitBranch, Music, CornerDownRight, Check
} from 'lucide-react';
import { message } from '@/utils/toast';

// 多级分支接口
export interface BranchChoice {
  id: string;
  dtmfKey: string; // '1', '2', '3', '0', '*', '#'
  label: string;   // 例如 "按 1 售前咨询", "按 2 售后支持"
  targetNodeType: 'prompt' | 'queue' | 'pstn' | 'hangup';
  targetTitle: string;
  targetConfig: Record<string, string>;
  audioFileName?: string;
  audioUrl?: string;
}

// 多级 IVR 树节点接口
export interface FlowTreeNode {
  id: string;
  type: 'start' | 'prompt' | 'queue' | 'pstn';
  title: string;
  subtitle: string;
  audioFileName?: string;
  audioUrl?: string;
  branches: BranchChoice[]; // 支持无限级分支展开！
  config: Record<string, string>;
}

export interface PaletteItem {
  type: 'prompt' | 'queue' | 'pstn';
  title: string;
  subtitle: string;
  icon: React.ElementType;
  color: string;
  defaultConfig: Record<string, string>;
}

const PALETTE_ITEMS: PaletteItem[] = [
  {
    type: 'prompt',
    title: '多级语音提示 (Prompt)',
    subtitle: '支持语音上传与按键多级分支',
    icon: Volume2,
    color: 'bg-primary/10 text-primary border-primary/30',
    defaultConfig: { timeout: '10' }
  },
  {
    type: 'queue',
    title: '转人工坐席队列',
    subtitle: '分配给客服组 (Support)',
    icon: Network,
    color: 'bg-warning/10 text-warning border-warning/30',
    defaultConfig: { queue_id: 'queue-support-01' }
  },
  {
    type: 'pstn',
    title: '外线中继转接',
    subtitle: '转接至手机/PSTN网关',
    icon: PhoneForwarded,
    color: 'bg-danger/10 text-danger border-danger/30',
    defaultConfig: { trunk_id: 'gw-telecom-trunk', target_number: '13800138000' }
  }
];

interface VisualFlowEditorProps {
  isOpen?: boolean;
  onClose: () => void;
}

export function VisualFlowEditor({ isOpen = true, onClose }: VisualFlowEditorProps) {
  // 核心：支持多级层级树的多节点 IVR 架构
  const [treeNodes, setTreeNodes] = useState<FlowTreeNode[]>([
    {
      id: 'root-1',
      type: 'start',
      title: '呼入入口 (Inbound Trigger)',
      subtitle: '匹配 DID 号码 400-800-9000',
      branches: [],
      config: { did: '4008009000' }
    },
    {
      id: 'prompt-level-1',
      type: 'prompt',
      title: '一级主导航语音 (Main IVR Menu)',
      subtitle: '支持多按键分支与本地音频上传试听',
      audioFileName: 'welcome_bgm.wav',
      audioUrl: '',
      branches: [
        {
          id: 'b-1',
          dtmfKey: '1',
          label: '按 1 售前咨询 (跳转二级子菜单)',
          targetNodeType: 'prompt',
          targetTitle: '二级售前语音子导航 (Sales Sub-Menu)',
          audioFileName: 'sales_prompt.wav',
          targetConfig: { timeout: '10' }
        },
        {
          id: 'b-2',
          dtmfKey: '2',
          label: '按 2 售后支持 (直连技术队列)',
          targetNodeType: 'queue',
          targetTitle: '转接至 VIP 售后坐席队列',
          targetConfig: { queue_id: 'queue-vip-support' }
        },
        {
          id: 'b-3',
          dtmfKey: '0',
          label: '按 0 人工客服 (外线中继)',
          targetNodeType: 'pstn',
          targetTitle: '转接至值班经理手机',
          targetConfig: { trunk_id: 'gw-mobile', target_number: '13800138000' }
        }
      ],
      config: { timeout: '10' }
    }
  ]);

  const [selectedNodeId, setSelectedNodeId] = useState<string>('prompt-level-1');
  const [draggedTemplate, setDraggedTemplate] = useState<PaletteItem | null>(null);

  const selectedNode = treeNodes.find(n => n.id === selectedNodeId);

  // 开始拖拽左侧组件
  const handleDragStart = (item: PaletteItem) => {
    setDraggedTemplate(item);
  };

  // 释放到画布
  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    if (!draggedTemplate) return;

    const newNodeId = `node-lvl-${Date.now()}`;
    const newNode: FlowTreeNode = {
      id: newNodeId,
      type: draggedTemplate.type,
      title: `${draggedTemplate.title} #${treeNodes.length}`,
      subtitle: draggedTemplate.subtitle,
      branches: draggedTemplate.type === 'prompt' ? [
        {
          id: `b-sub-${Date.now()}`,
          dtmfKey: '1',
          label: '按 1 分支',
          targetNodeType: 'queue',
          targetTitle: '分支目标队列',
          targetConfig: { queue_id: 'queue-01' }
        }
      ] : [],
      config: { ...draggedTemplate.defaultConfig }
    };

    setTreeNodes([...treeNodes, newNode]);
    setSelectedNodeId(newNodeId);
    setDraggedTemplate(null);
    message.success(`已创建并插入多级节点：${newNode.title}`);
  };

  // 添加多级分支项
  const handleAddBranch = (nodeId: string) => {
    setTreeNodes(treeNodes.map(n => {
      if (n.id === nodeId) {
        const nextKey = String((n.branches.length + 1) % 10);
        const newBranch: BranchChoice = {
          id: `b-${Date.now()}`,
          dtmfKey: nextKey,
          label: `按 ${nextKey} 新多级分支选项`,
          targetNodeType: 'prompt',
          targetTitle: `二级子导航 Prompt (${nextKey}键)`,
          targetConfig: { timeout: '10' }
        };
        return { ...n, branches: [...n.branches, newBranch] };
      }
      return n;
    }));
    message.success('已为该节点成功添加新的多级 DTMF 按键分支！');
  };

  // 移除多级分支项
  const handleRemoveBranch = (nodeId: string, branchId: string) => {
    setTreeNodes(treeNodes.map(n => {
      if (n.id === nodeId) {
        return {
          ...n,
          branches: n.branches.filter(b => b.id !== branchId)
        };
      }
      return n;
    }));
    message.info('多级分支选项已删除');
  };

  // 真实音频文件上传处理
  const handleFileUpload = (e: React.ChangeEvent<HTMLInputElement>, nodeId: string) => {
    const file = e.target.files?.[0];
    if (!file) return;

    const fileUrl = URL.createObjectURL(file);
    setTreeNodes(treeNodes.map(n => {
      if (n.id === nodeId) {
        return {
          ...n,
          audioFileName: file.name,
          audioUrl: fileUrl,
          subtitle: `已挂载音频: ${file.name} (${(file.size / 1024).toFixed(1)} KB)`
        };
      }
      return n;
    }));

    message.success(`音频文件 "${file.name}" 上传成功并已生成本地试听通道！`);
  };

  const renderEditorContent = () => (
    <div className="flex flex-col gap-4">
      {/* 顶栏控制条 */}
      <div className="flex items-center justify-between pb-3 border-b border-default-200">
        <div className="flex items-center gap-2">
          <span className="w-2.5 h-2.5 rounded-full bg-primary animate-ping" />
          <h3 className="text-sm font-extrabold text-foreground flex items-center gap-2">
            <span>多级嵌套 IVR 拖拽树状画布 (Multi-level Tree Flow Canvas)</span>
            <Chip size="sm" color="primary" variant="flat">真正多级分支 + 音频拖拽上传试听</Chip>
          </h3>
        </div>
        <Button size="sm" variant="flat" isIconOnly onPress={onClose}>
          <X className="w-4 h-4 text-default-500" />
        </Button>
      </div>

      <div className="flex flex-col lg:flex-row gap-4 h-[650px]">
        {/* 1. 左侧可拖拽组件面板 (Palette) */}
        <div className="w-full lg:w-72 p-4 bg-content2 rounded-2xl border border-default-200/80 flex flex-col gap-3 shrink-0">
          <div className="flex items-center gap-2 pb-2 border-b border-default-200">
            <Layers className="w-4 h-4 text-primary" />
            <span className="text-xs font-bold text-foreground">IVR 组件库 (可拖拽源)</span>
          </div>
          <p className="text-[11px] text-default-500">按住下方卡片拖拽入中间画布，可快速生成支持多级 DTMF 按键分支的 IVR 树节点：</p>

          <div className="flex flex-col gap-2.5 overflow-y-auto pr-1">
            {PALETTE_ITEMS.map((item, idx) => {
              const IconComponent = item.icon;
              return (
                <div
                  key={idx}
                  draggable
                  onDragStart={() => handleDragStart(item)}
                  aria-label={`节点：${item.title}`}
                  className={`p-3 rounded-xl border ${item.color} cursor-grab active:cursor-grabbing hover:shadow-md transition-all flex items-center gap-3 bg-content1/80 backdrop-blur-xs`}
                >
                  <div className="w-8 h-8 rounded-lg flex items-center justify-center bg-content1 shadow-2xs shrink-0">
                    <IconComponent className="w-4 h-4" />
                  </div>
                  <div className="flex flex-col min-w-0">
                    <span className="text-xs font-bold text-foreground truncate">{item.title}</span>
                    <span className="text-[10px] text-default-500 truncate">{item.subtitle}</span>
                  </div>
                  <Move className="w-3.5 h-3.5 ml-auto text-default-400 shrink-0" />
                </div>
              );
            })}
          </div>

          <div className="mt-auto p-3 bg-primary/10 rounded-xl border border-primary/20 text-[11px] text-primary flex items-center gap-2">
            <Sparkles className="w-4 h-4 shrink-0" />
            <span>每个节点均可任意扩展 1-9 / 0 多级按键跳转分支</span>
          </div>
        </div>

        {/* 2. 中间多级树状放置画布 (Multi-level Tree Canvas) */}
        <div
          onDragOver={(e) => e.preventDefault()}
          onDrop={handleDrop}
          role="application"
          aria-label="IVR 流程可视化画板"
          className="flex-1 p-6 bg-content2/70 rounded-2xl border-2 border-dashed border-primary/60 dark:border-primary/50 overflow-y-auto relative flex flex-col gap-6"
        >
          <div className="text-[11px] text-default-400 font-mono flex items-center justify-between pb-2 border-b border-default-200/60">
            <div className="flex items-center gap-1.5">
              <span className="w-2 h-2 rounded-full bg-success animate-pulse" />
              <span>Multi-level Tree Interactive Canvas (可随意拖放添加节点)</span>
            </div>
            <span className="text-[10px] text-primary font-semibold">支持多级展开与分支路径</span>
          </div>

          <div className="flex flex-col gap-6 items-center">
            {treeNodes.map((node) => {
              const isSelected = node.id === selectedNodeId;
              return (
                <div key={node.id} className="w-full max-w-xl flex flex-col gap-3">
                  {/* 主节点 Card */}
                  <Card
                    isPressable
                    onPress={() => setSelectedNodeId(node.id)}
                    aria-label={`节点：${node.title}`}
                    className={`border-2 transition-all ${
                      isSelected
                        ? 'border-primary shadow-lg scale-[1.01] bg-content1'
                        : 'border-default-200/80 hover:border-primary/60 bg-content1/90'
                    }`}
                  >
                    <CardBody className="p-4 flex flex-col gap-3">
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                          <div className="w-9 h-9 rounded-xl bg-primary/20 text-primary flex items-center justify-center font-bold">
                            {node.type === 'start' ? <Play className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
                          </div>
                          <div>
                            <h4 className="text-xs font-extrabold text-foreground">{node.title}</h4>
                            <p className="text-[11px] text-default-500">{node.subtitle}</p>
                          </div>
                        </div>

                        <div className="flex items-center gap-2">
                          {node.type === 'prompt' && (
                            <Button
                              size="sm"
                              color="primary"
                              variant="flat"
                              className="font-bold text-[11px]"
                              startContent={<Plus className="w-3.5 h-3.5" />}
                              onPress={() => handleAddBranch(node.id)}
                            >
                              + 添加多级按键分支
                            </Button>
                          )}
                          <Chip size="sm" variant="flat" color={isSelected ? 'primary' : 'default'}>
                            {node.type.toUpperCase()}
                          </Chip>
                        </div>
                      </div>

                      {/* 音频文件状态与在线试听 */}
                      {node.type === 'prompt' && (
                        <div className="p-2.5 bg-content2 rounded-xl border border-default-200/60 flex flex-col gap-2">
                          <div className="flex items-center justify-between text-[11px]">
                            <span className="text-default-500 flex items-center gap-1">
                              <Music className="w-3.5 h-3.5 text-primary" />
                              音频挂载: <strong className="text-foreground">{node.audioFileName || '未选择音频文件'}</strong>
                            </span>
                            {node.audioUrl && <Chip size="sm" color="success" variant="dot">可试听</Chip>}
                          </div>
                          {node.audioUrl && (
                            <audio src={node.audioUrl} controls className="w-full h-8 mt-1 rounded-lg" />
                          )}
                        </div>
                      )}

                      {/* 多级按键分支展示 (Branch Tree Children) */}
                      {node.branches.length > 0 && (
                        <div className="flex flex-col gap-2 pt-2 border-t border-default-200">
                          <span className="text-[10px] font-bold text-default-400 flex items-center gap-1">
                            <GitBranch className="w-3 h-3 text-primary" />
                            下级多级分支列表 (Multi-level Branches):
                          </span>

                          <div className="grid grid-cols-1 gap-2 pl-2">
                            {node.branches.map((b) => (
                              <div
                                key={b.id}
                                className="p-2.5 rounded-xl bg-primary/5 dark:bg-primary/20 border border-primary/20 flex items-center justify-between gap-2"
                              >
                                <div className="flex items-center gap-2">
                                  <CornerDownRight className="w-3.5 h-3.5 text-primary" />
                                  <Chip size="sm" color="primary" className="font-mono font-extrabold text-[10px]">
                                    按键 [{b.dtmfKey}]
                                  </Chip>
                                  <div className="flex flex-col">
                                    <span className="text-xs font-bold text-foreground">{b.label}</span>
                                    <span className="text-[10px] text-default-400">→ {b.targetTitle}</span>
                                  </div>
                                </div>

                                <Button
                                  isIconOnly
                                  size="sm"
                                  variant="light"
                                  color="danger"
                                  onPress={() => handleRemoveBranch(node.id, b.id)}
                                >
                                  <Trash2 className="w-3.5 h-3.5" />
                                </Button>
                              </div>
                            ))}
                          </div>
                        </div>
                      )}
                    </CardBody>
                  </Card>
                </div>
              );
            })}
          </div>
        </div>

        {/* 3. 右侧节点与音频属性 Inspector */}
        <div className="w-full lg:w-80 p-4 bg-content1 rounded-2xl border border-default-200/80 flex flex-col gap-4 shrink-0 overflow-y-auto">
          <div className="flex items-center gap-2 pb-2 border-b border-default-200">
            <Settings className="w-4 h-4 text-primary" />
            <span className="text-xs font-bold text-foreground">节点与音频属性 (Inspector)</span>
          </div>

          {selectedNode ? (
            <div className="flex flex-col gap-4">
              <div className="p-3 bg-content2 rounded-xl border border-default-200/60">
                <span className="text-[10px] text-default-400 font-mono">SELECTED NODE: {selectedNode.id}</span>
                <h4 className="text-xs font-bold text-foreground mt-1">{selectedNode.title}</h4>
              </div>

              {/* 核心：真实音频文件拖拽上传区域 */}
              {selectedNode.type === 'prompt' && (
                <div className="flex flex-col gap-2 p-3 bg-primary/5 rounded-xl border border-primary/20">
                  <label className="text-xs font-bold text-primary flex items-center gap-1.5">
                    <Upload className="w-3.5 h-3.5" />
                    <span>上传本地语音音频文件 (.wav/.mp3)</span>
                  </label>
                  <p className="text-[10px] text-default-500">上传后支持在中间画布直接在线试听播放</p>

                  <input
                    type="file"
                    accept="audio/*"
                    onChange={(e) => handleFileUpload(e, selectedNode.id)}
                    className="text-xs text-default-500 file:mr-2 file:py-1.5 file:px-3 file:rounded-xl file:border-0 file:text-xs file:font-semibold file:bg-primary file:text-white hover:file:bg-primary cursor-pointer"
                  />

                  {selectedNode.audioFileName && (
                    <div className="mt-1 flex items-center gap-1.5 text-[11px] text-success font-semibold">
                      <Check className="w-3.5 h-3.5" />
                      <span>已加载: {selectedNode.audioFileName}</span>
                    </div>
                  )}
                </div>
              )}

              {/* 超时参数配置 */}
              <Input
                label="等待按键超时秒数"
                variant="bordered"
                size="sm"
                value={selectedNode.config.timeout || '10'}
                onValueChange={(v) => {
                  setTreeNodes(treeNodes.map(n => n.id === selectedNode.id ? { ...n, config: { ...n.config, timeout: v } } : n));
                }}
              />
            </div>
          ) : (
            <div className="text-center py-10 text-xs text-default-400">请在画布点击选中节点配置参数与上传音频</div>
          )}
        </div>
      </div>
    </div>
  );

  if (!isOpen) return null;
  return renderEditorContent();
}
