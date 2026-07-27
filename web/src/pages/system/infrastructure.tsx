// 系统管理 - 基础设施节点管理
// 从 console.tsx 拆分

import { useCallback, useEffect, useState } from 'react';
import {
  Button, Card, CardBody, Chip,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell,
} from '@heroui/react';
import { RefreshCw, Server, Activity, Radio } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { ConfirmDialog } from '@/pages/shared/resource-workspace';
import { valueText } from '@/pages/shared/format';
import { message } from '@/utils/toast';
import type { Entity } from '@/services/resources';

interface MediaNode {
  id: string;
  type: string;
  advertised_addr: string;
  port_min: number;
  port_max: number;
  weight: number;
  control_url?: string;
  control_token_configured: boolean;
}

interface MediaCluster {
  allocation_strategy: string;
  health_check_interval_secs: number;
  unhealthy_threshold: number;
  nodes: MediaNode[];
}

interface RtcpQuality {
  last_rtt_ms: number | null;
  last_jitter: number | null;
  last_fraction_lost: number | null;
}

interface RtcpWindow {
  mos_x100: number | null;
  average_jitter: number | null;
  average_rtt_ms: number | null;
  samples: number;
}

interface MediaMetrics {
  forwarded_packets: number;
  received_packets: number;
  dropped_no_target_packets: number;
  dropped_invalid_packets: number;
  dtmf_events: number;
  recorded_packets: number;
  recording_queue_depth: number;
  recording_queue_capacity: number;
  recording_workers: number;
  rtcp_quality: RtcpQuality;
  rtcp_window: RtcpWindow;
  rtcp_quality_degraded: boolean;
}

export function InfrastructurePage() {
  const [sip, setSip] = useState<Entity>({});
  const [media, setMedia] = useState<MediaCluster | null>(null);
  const [metrics, setMetrics] = useState<MediaMetrics | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [drainRow, setDrainRow] = useState<Entity | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [sipData, mediaData, metricsData] = await Promise.all([
        api.get<Entity>('/infrastructure/sip-cluster'),
        api.get<MediaCluster>('/infrastructure/media-cluster'),
        api.get<MediaMetrics>('/infrastructure/media/metrics'),
      ]);
      setSip(sipData);
      setMedia(mediaData);
      setMetrics(metricsData);
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
  const mediaNodes: MediaNode[] = media?.nodes ?? [];

  // MOS 分数 (mos_x100 = MOS * 100)
  const mosScore = metrics?.rtcp_window?.mos_x100 != null
    ? (metrics.rtcp_window.mos_x100 / 100).toFixed(2)
    : null;
  const avgJitter = metrics?.rtcp_window?.average_jitter != null
    ? Math.round(metrics.rtcp_window.average_jitter)
    : null;
  const avgRtt = metrics?.rtcp_window?.average_rtt_ms != null
    ? Math.round(metrics.rtcp_window.average_rtt_ms)
    : null;
  const lastRtt = metrics?.rtcp_quality?.last_rtt_ms;
  const lastJitter = metrics?.rtcp_quality?.last_jitter;
  const lostFrac = metrics?.rtcp_quality?.last_fraction_lost;
  const lostPct = lostFrac != null ? ((lostFrac / 256) * 100).toFixed(1) : null;
  const qualityDegraded = metrics?.rtcp_quality_degraded ?? false;

  const queuePct = metrics
    ? Math.round((metrics.recording_queue_depth / metrics.recording_queue_capacity) * 100)
    : 0;

  return (
    <div className="flex flex-col gap-6">
      <Card shadow="sm" className="dash-enter overflow-hidden">
        <CardBody className="p-0">
          <div className="relative bg-gradient-to-br from-primary/10 via-content1 to-content1 px-6 py-5">
            <div className="flex flex-wrap items-center justify-between gap-4 relative z-10">
              <div className="min-w-0">
                <div className="flex items-center gap-2 mb-1.5">
                  <h1 className="text-xl font-bold text-foreground tracking-tight">电信软交换运行总览</h1>
                  <Chip color="success" size="sm" variant="flat" startContent={<span className="w-2 h-2 rounded-full bg-success animate-pulse" />}>
                    LIVE
                  </Chip>
                </div>
                <p className="text-tiny text-default-500">
                  实时信令事务 · 24h 话务趋势 · QoS 媒体质量 · 集群容量监测
                </p>
              </div>
              <div className="flex items-center gap-4">
                <div className="text-right hidden sm:block">
                  <div className="text-tiny text-default-400 font-mono tnum">
                    {new Date().toLocaleDateString('zh-CN')} {new Date().toLocaleTimeString('zh-CN', { hour12: false })}
                  </div>
                  <div className="text-[10px] text-default-400">
                    自动刷新 · 10s
                  </div>
                </div>
                <Button
                  variant="flat"
                  size="sm"
                  isLoading={loading}
                  onPress={load}
                  startContent={<RefreshCw className="w-4 h-4" />}
                >
                  刷新
                </Button>
              </div>
            </div>
          </div>
        </CardBody>
      </Card>

      {error ? (
        <ErrorState error={error} retry={load} />
      ) : (
        <div className="flex flex-col gap-6">

          {/* SIP 节点表格 */}
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <Server className="w-4 h-4 text-success" />
              <h3 className="text-sm font-bold text-foreground">SIP 控制面代理节点</h3>
              <Chip size="sm" variant="flat" color="default" className="font-mono">{sipNodes.length} 节点</Chip>
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

          {/* 媒体节点表格 */}
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <Radio className="w-4 h-4 text-primary" />
              <h3 className="text-sm font-bold text-foreground">RTP 媒体转发节点</h3>
              <Chip size="sm" variant="flat" color="default" className="font-mono">{mediaNodes.length} 节点</Chip>
              {media?.allocation_strategy && (
                <Chip size="sm" variant="flat" color="primary">{media.allocation_strategy}</Chip>
              )}
            </div>

            <Table aria-label="媒体节点列表" isStriped>
              <TableHeader>
                <TableColumn key="id">节点 ID</TableColumn>
                <TableColumn key="type">类型</TableColumn>
                <TableColumn key="advertised_addr">媒体地址</TableColumn>
                <TableColumn key="ports">RTP 端口范围</TableColumn>
                <TableColumn key="weight">调度权重</TableColumn>
                <TableColumn key="control">控制通道</TableColumn>
              </TableHeader>
              <TableBody items={mediaNodes} emptyContent="暂无媒体节点">
                {(node) => (
                  <TableRow key={node.id}>
                    <TableCell><span className="font-mono font-bold text-foreground">{node.id}</span></TableCell>
                    <TableCell>
                      <Chip size="sm" variant="flat" color={node.type === 'local' ? 'success' : 'primary'}>
                        {node.type}
                      </Chip>
                    </TableCell>
                    <TableCell><span className="font-mono text-default-600">{node.advertised_addr}</span></TableCell>
                    <TableCell>
                      <span className="font-mono text-xs text-default-500">
                        {node.port_min} – {node.port_max}
                        <span className="text-default-400 ml-1">({node.port_max - node.port_min} 路)</span>
                      </span>
                    </TableCell>
                    <TableCell>
                      <Chip size="sm" variant="bordered" className="font-mono">{node.weight}</Chip>
                    </TableCell>
                    <TableCell>
                      {node.control_url ? (
                        <div className="flex items-center gap-1.5">
                          <span className="font-mono text-xs text-default-500">{node.control_url}</span>
                          <Chip size="sm" color={node.control_token_configured ? 'success' : 'warning'} variant="flat">
                            {node.control_token_configured ? '已配置 Token' : '未配 Token'}
                          </Chip>
                        </div>
                      ) : (
                        <Chip size="sm" variant="flat" color="default">进程内直连</Chip>
                      )}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>

          {/* 实时 RTP 质量指标 */}
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              <Activity className="w-4 h-4 text-warning" />
              <h3 className="text-sm font-bold text-foreground">RTP 媒体质量实时指标</h3>
              {qualityDegraded
                ? <Chip size="sm" color="danger" variant="flat">质量劣化告警</Chip>
                : <Chip size="sm" color="success" variant="flat">质量正常</Chip>}
            </div>

            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
              {/* MOS */}
              <Card shadow="sm">
                <CardBody className="p-4 flex flex-col items-center gap-1">
                  <span className="text-[10px] text-default-400 uppercase tracking-wide">平均 MOS 音质</span>
                  <span className={`text-2xl font-extrabold font-mono ${mosScore !== null ? (parseFloat(mosScore) >= 4.0 ? 'text-success' : parseFloat(mosScore) >= 3.5 ? 'text-warning' : 'text-danger') : 'text-default-300'}`}>
                    {mosScore !== null ? `${mosScore}` : '—'}
                  </span>
                  <span className="text-[10px] text-default-400">/ 5.0  ({metrics?.rtcp_window?.samples ?? 0} 样本)</span>
                </CardBody>
              </Card>

              {/* RTT */}
              <Card shadow="sm">
                <CardBody className="p-4 flex flex-col items-center gap-1">
                  <span className="text-[10px] text-default-400 uppercase tracking-wide">往返时延 RTT</span>
                  <span className={`text-2xl font-extrabold font-mono ${lastRtt != null ? (lastRtt < 50 ? 'text-success' : lastRtt < 150 ? 'text-warning' : 'text-danger') : 'text-default-300'}`}>
                    {lastRtt != null ? `${Math.round(lastRtt)}` : avgRtt != null ? `${avgRtt}` : '—'}
                  </span>
                  <span className="text-[10px] text-default-400">ms  (均值: {avgRtt != null ? `${avgRtt}ms` : '—'})</span>
                </CardBody>
              </Card>

              {/* Jitter */}
              <Card shadow="sm">
                <CardBody className="p-4 flex flex-col items-center gap-1">
                  <span className="text-[10px] text-default-400 uppercase tracking-wide">Jitter 抖动</span>
                  <span className={`text-2xl font-extrabold font-mono ${lastJitter != null ? (lastJitter < 30 ? 'text-success' : lastJitter < 80 ? 'text-warning' : 'text-danger') : 'text-default-300'}`}>
                    {lastJitter != null ? Math.round(lastJitter) : avgJitter != null ? avgJitter : '—'}
                  </span>
                  <span className="text-[10px] text-default-400">ms  (均值: {avgJitter != null ? `${avgJitter}ms` : '—'})</span>
                </CardBody>
              </Card>

              {/* 丢包率 */}
              <Card shadow="sm">
                <CardBody className="p-4 flex flex-col items-center gap-1">
                  <span className="text-[10px] text-default-400 uppercase tracking-wide">丢包率</span>
                  <span className={`text-2xl font-extrabold font-mono ${lostPct != null ? (parseFloat(lostPct) < 1 ? 'text-success' : parseFloat(lostPct) < 3 ? 'text-warning' : 'text-danger') : 'text-default-300'}`}>
                    {lostPct != null ? `${lostPct}%` : '—'}
                  </span>
                  <span className="text-[10px] text-default-400">RTCP 最后一报</span>
                </CardBody>
              </Card>
            </div>

            {/* RTP 包统计 */}
            <div className="grid grid-cols-2 md:grid-cols-3 gap-4">
              <Card shadow="sm">
                <CardBody className="p-4 flex flex-col gap-2">
                  <span className="text-xs font-bold text-default-500">RTP 包转发统计</span>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">已转发</span>
                    <span className="font-mono font-bold text-success">{(metrics?.forwarded_packets ?? 0).toLocaleString()}</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">已接收</span>
                    <span className="font-mono">{(metrics?.received_packets ?? 0).toLocaleString()}</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">无路由丢弃</span>
                    <span className={`font-mono ${(metrics?.dropped_no_target_packets ?? 0) > 0 ? 'text-warning' : 'text-default-400'}`}>
                      {(metrics?.dropped_no_target_packets ?? 0).toLocaleString()}
                    </span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">非法包丢弃</span>
                    <span className={`font-mono ${(metrics?.dropped_invalid_packets ?? 0) > 0 ? 'text-danger' : 'text-default-400'}`}>
                      {(metrics?.dropped_invalid_packets ?? 0).toLocaleString()}
                    </span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">DTMF 事件</span>
                    <span className="font-mono">{(metrics?.dtmf_events ?? 0).toLocaleString()}</span>
                  </div>
                </CardBody>
              </Card>

              <Card shadow="sm">
                <CardBody className="p-4 flex flex-col gap-2">
                  <span className="text-xs font-bold text-default-500">录音队列状态</span>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">录音包数</span>
                    <span className="font-mono font-bold text-primary">{(metrics?.recorded_packets ?? 0).toLocaleString()}</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">Worker 数</span>
                    <span className="font-mono">{metrics?.recording_workers ?? 0}</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">队列深度</span>
                    <span className={`font-mono ${queuePct > 80 ? 'text-danger' : queuePct > 50 ? 'text-warning' : 'text-default-400'}`}>
                      {metrics?.recording_queue_depth ?? 0} / {metrics?.recording_queue_capacity ?? 0}
                    </span>
                  </div>
                  <div className="w-full bg-content2 rounded-full h-1.5 mt-1">
                    <div
                      className={`h-1.5 rounded-full transition-all ${queuePct > 80 ? 'bg-danger' : queuePct > 50 ? 'bg-warning' : 'bg-success'}`}
                      style={{ width: `${queuePct}%` }}
                    />
                  </div>
                  <span className="text-[10px] text-default-400 text-center">{queuePct}% 使用率</span>
                </CardBody>
              </Card>

              <Card shadow="sm">
                <CardBody className="p-4 flex flex-col gap-2">
                  <span className="text-xs font-bold text-default-500">媒体集群配置</span>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">调度策略</span>
                    <Chip size="sm" variant="flat" color="primary" className="text-[10px]">
                      {media?.allocation_strategy ?? '—'}
                    </Chip>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">健康检查间隔</span>
                    <span className="font-mono">{media?.health_check_interval_secs ?? '—'} s</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">故障阈值</span>
                    <span className="font-mono">{media?.unhealthy_threshold ?? '—'} 次</span>
                  </div>
                  <div className="flex justify-between text-xs">
                    <span className="text-default-400">节点总数</span>
                    <span className="font-mono font-bold">{mediaNodes.length}</span>
                  </div>
                </CardBody>
              </Card>
            </div>
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
