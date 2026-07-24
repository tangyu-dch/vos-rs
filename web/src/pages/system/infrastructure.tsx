// 系统管理 - 基础设施节点管理
// 从 console.tsx 拆分

import { useCallback, useEffect, useState } from 'react';
import {
  Button, Card, CardBody, Chip,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell,
} from '@heroui/react';
import { RefreshCw, Server, Activity, Sparkles } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { ConfirmDialog } from '@/pages/shared/resource-workspace';
import { valueText } from '@/pages/shared/format';
import { message } from '@/utils/toast';
import type { Entity } from '@/services/resources';

export function InfrastructurePage() {
  const [sip, setSip] = useState<Entity>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [drainRow, setDrainRow] = useState<Entity | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const sipData = await api.get<Entity>('/infrastructure/sip-cluster');
      setSip(sipData);
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const control = async (id: string, action: 'drain' | 'resume') => {
    try {
      setSaving(true);
      await api.post(`/infrastructure/sip-cluster/nodes/${encodeURIComponent(id)}/${action}`);
      message.success(action === 'drain' ? '节点已成功摘流' : '节点已成功恢复上线');
      await load();
    } catch (e) {
      message.error(e instanceof Error ? e.message : '操作失败');
    } finally {
      setSaving(false);
    }
  };

  const sipNodes = Array.isArray(sip.nodes) ? (sip.nodes as Entity[]) : [];

  return (
    <div className="flex flex-col gap-6">
      <Card shadow="sm" className="p-2">
        <CardBody className="p-4 flex flex-wrap items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-2 mb-1">
              <h2 className="text-base font-bold text-foreground">软交换集群节点管理</h2>
              <Chip color="success" size="sm" variant="flat">高可用架构 Active-Active</Chip>
            </div>
            <p className="text-tiny text-default-500">实时查看 SIP 边缘代理节点、媒体转发集群健康度与节点摘流控制</p>
          </div>
          <Button variant="flat" size="sm" isLoading={loading} onPress={load} startContent={<RefreshCw className="w-4 h-4" />}>
            刷新节点
          </Button>
        </CardBody>
      </Card>

      {error ? (
        <ErrorState error={error} retry={load} />
      ) : (
        <div className="flex flex-col gap-6">
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <Server className="w-4 h-4 text-success" />
              <h3 className="text-sm font-bold text-foreground">SIP 控制面代理节点</h3>
            </div>

            <Table aria-label="SIP 节点列表" isStriped>
              <TableHeader>
                <TableColumn key="node_id">节点名称</TableColumn>
                <TableColumn key="advertised_addr">通告 SIP 地址</TableColumn>
                <TableColumn key="status">节点状态</TableColumn>
                <TableColumn key="active_calls">活跃并发通话</TableColumn>
                <TableColumn key="version">固件版本</TableColumn>
                <TableColumn key="actions" align="end">节点控制</TableColumn>
              </TableHeader>
              <TableBody items={sipNodes} emptyContent="暂无在线 SIP 节点">
                {(node) => (
                  <TableRow key={String(node.node_id)}>
                    <TableCell><span className="font-mono font-bold text-foreground">{valueText(node.node_id)}</span></TableCell>
                    <TableCell><span className="font-mono text-default-600">{valueText(node.advertised_addr)}</span></TableCell>
                    <TableCell>
                      <Chip
                        size="sm"
                        color={node.status === 'online' || node.status === 'active' ? 'success' : node.status === 'draining' ? 'warning' : 'danger'}
                        variant="flat"
                      >
                        {valueText(node.status)}
                      </Chip>
                    </TableCell>
                    <TableCell><span className="font-mono font-bold text-success">{valueText(node.active_calls)} CAPS</span></TableCell>
                    <TableCell>
                      <Chip size="sm" variant="bordered" className="font-mono">{valueText(node.version)}</Chip>
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center justify-end">
                        {node.status === 'draining' ? (
                          <Button size="sm" color="success" variant="flat" onPress={() => control(String(node.node_id), 'resume')}>
                            恢复服务
                          </Button>
                        ) : (
                          <Button size="sm" color="warning" variant="flat" onPress={() => setDrainRow(node)}>
                            优雅摘流
                          </Button>
                        )}
                      </div>
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            <Card shadow="sm" className="p-2">
              <CardBody className="p-5 flex flex-col gap-3">
                <div className="flex items-center justify-between pb-3 border-b border-divider">
                  <div className="flex items-center gap-2">
                    <Activity className="w-4 h-4 text-primary" />
                    <h3 className="text-sm font-bold text-foreground">中继网关 OPTIONS 探活</h3>
                    <Chip size="sm" variant="flat" color="warning">示例数据</Chip>
                  </div>
                  <Chip color="success" size="sm" variant="flat">心跳 3s/次</Chip>
                </div>
                <div className="flex flex-col gap-2.5">
                  <div className="flex items-center justify-between p-3 rounded-xl bg-success/10 border border-success/20">
                    <div className="flex flex-col">
                      <span className="text-xs font-bold text-foreground">Primary Trunk Gateway (落地主中继)</span>
                      <span className="text-[11px] font-mono text-default-500">192.168.1.10:5060 (UDP)</span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-mono font-bold text-success">RTT: 12ms</span>
                      <Chip color="success" size="sm">HEALTHY</Chip>
                    </div>
                  </div>
                  <div className="flex items-center justify-between p-3 rounded-xl bg-warning/10 border border-warning/20">
                    <div className="flex flex-col">
                      <span className="text-xs font-bold text-foreground">Backup Trunk Gateway (备用中继)</span>
                      <span className="text-[11px] font-mono text-default-500">10.0.0.8:5060 (UDP)</span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-mono font-bold text-warning">RTT: 45ms (丢包 1.2%)</span>
                      <Chip color="warning" size="sm" variant="flat">DEGRADED</Chip>
                    </div>
                  </div>
                </div>
              </CardBody>
            </Card>

            <Card shadow="sm" className="p-2">
              <CardBody className="p-5 flex flex-col gap-3">
                <div className="flex items-center justify-between pb-3 border-b border-divider">
                  <div className="flex items-center gap-2">
                    <Sparkles className="w-4 h-4 text-primary" />
                    <h3 className="text-sm font-bold text-foreground">媒体质量指标 (QoS & MOS)</h3>
                    <Chip size="sm" variant="flat" color="warning">示例数据</Chip>
                  </div>
                  <Chip color="primary" size="sm" variant="flat">Opus / G.711</Chip>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div className="p-3 bg-content2 rounded-xl border border-default-200/60 flex flex-col items-center">
                    <span className="text-[10px] text-default-400">平均 MOS 音质分</span>
                    <span className="text-xl font-extrabold text-success font-mono mt-1">4.38 / 5.0</span>
                    <span className="text-[10px] text-default-400 mt-1">电信级清晰音质</span>
                  </div>
                  <div className="p-3 bg-content2 rounded-xl border border-default-200/60 flex flex-col items-center">
                    <span className="text-[10px] text-default-400">Jitter 抖动缓冲区</span>
                    <span className="text-xl font-extrabold text-primary font-mono mt-1">15 ms</span>
                    <span className="text-[10px] text-default-400 mt-1">自适应缓冲机制</span>
                  </div>
                </div>
              </CardBody>
            </Card>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={Boolean(drainRow)}
        title="确认摘流"
        message="摘流后节点将拒绝接入新呼叫，确认摘流？"
        loading={saving}
        onConfirm={async () => {
          if (drainRow) await control(String(drainRow.node_id), 'drain');
          setDrainRow(null);
        }}
        onClose={() => setDrainRow(null)}
      />
    </div>
  );
}
