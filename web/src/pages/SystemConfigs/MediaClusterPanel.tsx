import { useCallback, useEffect, useState } from 'react';
import { Button, Input, InputNumber, Message, Select, Space, Spin, Tag } from '@arco-design/web-react';
import { IconDelete, IconPlus, IconRefresh, IconSave } from '@arco-design/web-react/icon';
import { apiService, type MediaClusterConfig, type MediaClusterNode } from '@/services/api';

const EMPTY_CONFIG: MediaClusterConfig = {
  allocation_strategy: 'weighted_round_robin',
  health_check_interval_secs: 3,
  unhealthy_threshold: 3,
  nodes: [],
};

function emptyNode(index: number): MediaClusterNode {
  const portMin = 40000 + index * 1000;
  return {
    id: `media-edge-${index + 1}`,
    type: 'remote',
    control_url: `http://127.0.0.1:${3030 + index}`,
    advertised_addr: '127.0.0.1',
    port_min: portMin,
    port_max: portMin + 998,
    weight: 1,
    control_token_configured: false,
  };
}

function validate(config: MediaClusterConfig): string | undefined {
  if (config.nodes.length === 0) return '至少需要配置一个媒体节点';
  const ids = new Set<string>();
  let localNodes = 0;
  for (let index = 0; index < config.nodes.length; index += 1) {
    const node = config.nodes[index];
    if (!node.id.trim() || !node.advertised_addr.trim()) return '节点标识和通告地址不能为空';
    if (ids.has(node.id)) return `节点标识重复：${node.id}`;
    ids.add(node.id);
    if (node.type === 'local') {
      localNodes += 1;
      if (localNodes > 1) return '最多只能配置一个本地媒体节点';
      if (node.control_url?.trim()) return `本地节点 ${node.id} 不能配置控制地址`;
    } else if (!node.control_url?.match(/^(https?:\/\/|uds:\/\/)/)) {
      return `远程节点 ${node.id} 的控制地址必须使用 http、https 或 uds`;
    }
    if (node.port_min < 1024 || node.port_min % 2 !== 0 || node.port_max % 2 !== 0 || node.port_max <= node.port_min) return `节点 ${node.id} 的 RTP 端口范围无效`;
    if (node.weight < 1) return `节点 ${node.id} 的权重必须大于零`;
    for (const other of config.nodes.slice(0, index)) {
      if (node.port_min <= other.port_max && other.port_min <= node.port_max) return `节点 ${node.id} 与 ${other.id} 的 RTP 端口范围重叠`;
    }
  }
  return undefined;
}

export default function MediaClusterPanel() {
  const [config, setConfig] = useState<MediaClusterConfig>(EMPTY_CONFIG);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async (notify = false) => {
    setLoading(true);
    try {
      setConfig(await apiService.getMediaCluster());
      if (notify) Message.success('媒体集群配置已刷新');
    } catch (error) {
      Message.error(error instanceof Error ? error.message : '加载媒体集群配置失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const updateNode = (index: number, changes: Partial<MediaClusterNode>) => {
    setConfig((current) => ({
      ...current,
      nodes: current.nodes.map((node, nodeIndex) => nodeIndex === index ? { ...node, ...changes } : node),
    }));
  };

  const save = async () => {
    const error = validate(config);
    if (error) { Message.error(error); return; }
    setSaving(true);
    try {
      setConfig(await apiService.updateMediaCluster(config));
      Message.success('媒体集群配置已保存，滚动重启 sip-edge 后生效');
    } catch (error) {
      Message.error(error instanceof Error ? error.message : '保存媒体集群配置失败');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Spin loading={loading} style={{ width: '100%' }}>
      <section className="system-configs__panel">
        <div className="system-configs__cluster-header">
          <div className="system-configs__intro">
            <h2>Media Edge 节点池</h2>
            <p>本地与远程媒体使用同一节点模型；所有媒体腿按 Call-ID 保持节点亲和，节点列表不能为空。</p>
          </div>
          <Space>
            <Button icon={<IconRefresh />} onClick={() => void load(true)}>刷新</Button>
            <Button icon={<IconPlus />} onClick={() => setConfig((value) => ({ ...value, nodes: [...value.nodes, emptyNode(value.nodes.length)] }))}>添加节点</Button>
            <Button type="primary" icon={<IconSave />} loading={saving} onClick={() => void save()}>保存节点池</Button>
          </Space>
        </div>
        <div className="system-configs__cluster-options">
          <label>分配策略<Select value={config.allocation_strategy} onChange={(value) => setConfig((current) => ({ ...current, allocation_strategy: value }))} options={[
            { label: '按权重轮询', value: 'weighted_round_robin' },
            { label: '最少活跃端点', value: 'least_sessions' },
            { label: 'Call-ID 稳定哈希', value: 'call_id_hash' },
          ]} /></label>
          <label>健康检查（秒）<InputNumber min={1} value={config.health_check_interval_secs} onChange={(value) => setConfig((current) => ({ ...current, health_check_interval_secs: Number(value) }))} /></label>
          <label>失败摘除阈值<InputNumber min={1} value={config.unhealthy_threshold} onChange={(value) => setConfig((current) => ({ ...current, unhealthy_threshold: Number(value) }))} /></label>
        </div>
        {config.nodes.length === 0 ? <div className="system-configs__cluster-empty">尚未配置媒体节点，保存和 sip-edge 启动都会失败。</div> : config.nodes.map((node, index) => (
          <div className="system-configs__cluster-node" key={`${node.id}-${index}`}>
            <div className="system-configs__cluster-node-title"><strong>节点 {index + 1}</strong>{node.control_token_configured && <Tag color="green">控制密钥已配置</Tag>}<Button status="danger" size="small" icon={<IconDelete />} onClick={() => setConfig((current) => ({ ...current, nodes: current.nodes.filter((_, nodeIndex) => nodeIndex !== index) }))}>删除</Button></div>
            <label>节点标识<Input value={node.id} onChange={(value) => updateNode(index, { id: value })} /></label>
            <label>节点类型<Select value={node.type} onChange={(value) => updateNode(index, value === 'local' ? { type: 'local', control_url: undefined, control_token: undefined, control_token_configured: false } : { type: 'remote' })} options={[
              { label: '本地（sip-edge 进程内）', value: 'local' },
              { label: '远程（独立 media-edge）', value: 'remote' },
            ]} /></label>
            {node.type === 'remote' && <label>控制地址<Input value={node.control_url ?? ''} onChange={(value) => updateNode(index, { control_url: value })} placeholder="http://10.0.1.11:3030" /></label>}
            <label>SDP 通告地址<Input value={node.advertised_addr} onChange={(value) => updateNode(index, { advertised_addr: value })} /></label>
            <label>RTP 起始端口<InputNumber min={1024} step={2} value={node.port_min} onChange={(value) => updateNode(index, { port_min: Number(value) })} /></label>
            <label>RTP 结束端口<InputNumber min={1024} step={2} value={node.port_max} onChange={(value) => updateNode(index, { port_max: Number(value) })} /></label>
            <label>权重<InputNumber min={1} value={node.weight} onChange={(value) => updateNode(index, { weight: Number(value) })} /></label>
            {node.type === 'remote' && <label className="system-configs__cluster-token">控制密钥<Input.Password value={node.control_token ?? ''} onChange={(value) => updateNode(index, { control_token: value })} placeholder={node.control_token_configured ? '留空保持原密钥' : '输入控制密钥'} /><Button size="mini" onClick={() => updateNode(index, { control_token: '', control_token_configured: false })}>清除密钥</Button></label>}
          </div>
        ))}
      </section>
    </Spin>
  );
}
