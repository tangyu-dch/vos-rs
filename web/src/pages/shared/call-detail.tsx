// 通话详情相关组件：SIP 流图、呼叫摘要、媒体诊断
// 从 console.tsx 拆分

import { useMemo, useState } from 'react';
import {
  Button,
  Card,
  CardBody,
  Chip,
  Modal,
  ModalContent,
  ModalHeader,
  ModalBody,
  ModalFooter,
  Progress,
} from '@heroui/react';
import type { CdrAuditSnapshot } from '@/types';
import type { Entity } from '@/services/resources';
import { callDetailText } from '@/pages/shared/format';
import { DetailFields } from '@/pages/shared/entity-detail';

export interface SipFlowEvent {
  timestamp_str?: string;
  diff_ms?: number;
  offset_ms: number;
  message: string;
  direction: string;
  from_addr?: string;
  to_addr?: string;
  note: string;
  raw_message?: string;
}

const PARTIES = ['UAC', 'B2BUA', 'UAS'] as const;
type Party = (typeof PARTIES)[number];

function directionToParties(direction: string): [Party, Party] {
  const map: Record<string, [Party, Party]> = {
    uac_to_b2bua: ['UAC', 'B2BUA'],
    b2bua_to_uac: ['B2BUA', 'UAC'],
    b2bua_to_uas: ['B2BUA', 'UAS'],
    uas_to_b2bua: ['UAS', 'B2BUA'],
  };
  return map[direction] ?? ['UAC', 'B2BUA'];
}

function parseSDP(rawMessage: string | undefined) {
  if (!rawMessage) return null;
  const sdpStartIndex = rawMessage.indexOf('v=0');
  if (sdpStartIndex === -1) return null;
  const sdp = rawMessage.slice(sdpStartIndex);
  const mediaLines = sdp.match(/^m=(audio|video)\s+(\d+)\s+[\w/]+\s+(.*)$/gm);
  if (!mediaLines) return null;

  const result: { mediaType: string; port: string; codecs: string[] }[] = [];
  mediaLines.forEach((mLine) => {
    const parts = mLine.trim().split(/\s+/);
    const mediaType = parts[0].substring(2);
    const port = parts[1];
    const payloads = parts.slice(3);

    const codecs: string[] = [];
    payloads.forEach((pt) => {
      const rtpmapRegex = new RegExp(`^a=rtpmap:${pt}\\s+(.+?)(?:/\\d+)?$`, 'm');
      const match = sdp.match(rtpmapRegex);
      if (match) {
        codecs.push(match[1]);
      } else {
        if (pt === '0') codecs.push('PCMU');
        else if (pt === '8') codecs.push('PCMA');
        else if (pt === '18') codecs.push('G729');
        else if (pt === '9') codecs.push('G722');
        else if (pt === '3') codecs.push('GSM');
        else if (pt === '101' || pt === '104') codecs.push('telephone-event');
      }
    });
    result.push({ mediaType, port, codecs });
  });
  return result;
}

export function SipFlowDiagram({ events }: { events: SipFlowEvent[] }) {
  const [selectedEvent, setSelectedEvent] = useState<SipFlowEvent | null>(null);
  const [hideRetransmissions, setHideRetransmissions] = useState(false);

  // Filter retransmissions if user toggles filter
  const displayedEvents = useMemo(() => {
    if (!hideRetransmissions) return events;
    return events.filter((e) => !e.message.includes('[重传]') && !e.note.includes('Re-tx'));
  }, [events, hideRetransmissions]);

  // Extract endpoints from events or defaults
  const uacAddr = useMemo(() => {
    const ev = events.find((e) => e.direction === 'uac_to_b2bua');
    return ev?.from_addr || '主叫 (UAC)';
  }, [events]);

  const b2buaAddr = useMemo(() => {
    const ev = events.find((e) => e.direction === 'uac_to_b2bua' || e.direction === 'b2bua_to_uac');
    return ev?.to_addr || '127.0.0.1:5060';
  }, [events]);

  const uasAddr = useMemo(() => {
    const ev = events.find((e) => e.direction === 'b2bua_to_uas' || e.direction === 'uas_to_b2bua');
    return ev?.to_addr || ev?.from_addr || '被叫 (UAS)';
  }, [events]);

  // Always display 3 columns (UAC, B2BUA, UAS) as in 图2
  const TIME_COL_W = 140;
  const COL_SPACING = 240;
  const columns: Record<Party, number> = {
    UAC: TIME_COL_W + 60,
    B2BUA: TIME_COL_W + 60 + COL_SPACING,
    UAS: TIME_COL_W + 60 + COL_SPACING * 2,
  };

  const ROW_H = 64;
  const HEADER_H = 50;
  const svgWidth = TIME_COL_W + 60 + COL_SPACING * 2 + 100;
  const svgHeight = HEADER_H + displayedEvents.length * ROW_H + 30;

  const parsedSDP = useMemo(() => parseSDP(selectedEvent?.raw_message), [selectedEvent]);

  return (
    <div
      className="bg-content1 text-foreground rounded-medium border border-divider p-4 font-mono select-none transition-colors"
      style={{ overflowX: 'auto' }}
    >
      <div className="flex justify-between items-center mb-3 text-tiny">
        <div className="flex items-center gap-2">
          <span className="font-semibold text-foreground">信令统计:</span>
          <Chip size="sm" variant="flat" color="primary">
            共 {events.length} 条报文
          </Chip>
          {events.some((e) => e.message.includes('[重传]')) && (
            <Chip size="sm" variant="flat" color="warning">
              检测到重传
            </Chip>
          )}
        </div>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1.5 cursor-pointer text-default-600 hover:text-foreground">
            <input
              type="checkbox"
              checked={hideRetransmissions}
              onChange={(e) => setHideRetransmissions(e.target.checked)}
              className="rounded border-default-400 text-primary focus:ring-primary"
            />
            隐藏 UDP 重传
          </label>
        </div>
      </div>

      <svg
        width={svgWidth}
        height={svgHeight}
        role="img"
        aria-label="SIP 信令时序图"
        className="text-foreground"
        style={{
          fontFamily: 'Consolas, Monaco, "Courier New", monospace',
          display: 'block',
          margin: '0 auto',
        }}
      >
        {/* Header Entity Labels */}
        <g key="headers">
          <text
            x={columns.UAC}
            y={24}
            textAnchor="middle"
            fill="currentColor"
            className="text-primary font-bold"
            fontSize={13}
          >
            {uacAddr}
          </text>
          <text
            x={columns.B2BUA}
            y={24}
            textAnchor="middle"
            fill="currentColor"
            className="text-success font-bold"
            fontSize={13}
          >
            {b2buaAddr}
          </text>
          <text
            x={columns.UAS}
            y={24}
            textAnchor="middle"
            fill="currentColor"
            className="text-warning font-bold"
            fontSize={13}
          >
            {uasAddr}
          </text>

          {/* Underline for Header */}
          <line
            x1={columns.UAC - 80}
            y1={36}
            x2={columns.UAC + 80}
            y2={36}
            stroke="currentColor"
            strokeOpacity={0.2}
            strokeWidth={1}
          />
          <line
            x1={columns.B2BUA - 80}
            y1={36}
            x2={columns.B2BUA + 80}
            y2={36}
            stroke="currentColor"
            strokeOpacity={0.2}
            strokeWidth={1}
          />
          <line
            x1={columns.UAS - 80}
            y1={36}
            x2={columns.UAS + 80}
            y2={36}
            stroke="currentColor"
            strokeOpacity={0.2}
            strokeWidth={1}
          />
        </g>

        {/* Time Column Separator Vertical Line */}
        <line
          x1={TIME_COL_W}
          y1={HEADER_H}
          x2={TIME_COL_W}
          y2={svgHeight - 10}
          stroke="currentColor"
          strokeOpacity={0.15}
          strokeWidth={1}
        />

        {/* Vertical Lifelines for 3 Entities */}
        {PARTIES.map((party) => (
          <line
            key={`line-${party}`}
            x1={columns[party]}
            y1={HEADER_H}
            x2={columns[party]}
            y2={svgHeight - 10}
            stroke="currentColor"
            strokeOpacity={0.15}
            strokeWidth={1}
          />
        ))}

        {/* Message Flows */}
        {displayedEvents.map((event, idx) => {
          const y = HEADER_H + idx * ROW_H + ROW_H / 2;
          const [from, to] = directionToParties(event.direction);
          const x1 = columns[from];
          const x2 = columns[to];
          const rightward = x2 > x1;
          const isRetransmission = event.message.includes('[重传]');

          // Theme-aware adaptive colors
          let msgColor = 'hsl(var(--heroui-primary))';
          if (isRetransmission) msgColor = 'hsl(var(--heroui-warning))';
          else if (/100|180/.test(event.message)) msgColor = 'hsl(var(--heroui-success))';
          else if (/200/.test(event.message)) msgColor = 'hsl(var(--heroui-success))';
          else if (/INVITE/.test(event.message)) msgColor = 'hsl(var(--heroui-primary))';
          else if (/ACK|BYE/.test(event.message)) msgColor = 'hsl(var(--heroui-warning))';
          else if (/4|5|6/.test(event.message)) msgColor = 'hsl(var(--heroui-danger))';

          const hasRaw = Boolean(event.raw_message);
          const diffStr =
            event.diff_ms !== undefined
              ? `+${event.diff_ms.toFixed(6)}`
              : `+${(event.offset_ms / 1000).toFixed(6)}`;

          return (
            <g
              key={idx}
              className="hover:opacity-80 transition-opacity"
              style={{ cursor: hasRaw ? 'pointer' : 'default' }}
              onClick={() => {
                if (hasRaw) setSelectedEvent(event);
              }}
            >
              {/* Left Column Timestamps */}
              <text
                x={TIME_COL_W - 12}
                y={y - 8}
                fontSize={11}
                fill="currentColor"
                opacity={0.6}
                textAnchor="end"
              >
                {event.timestamp_str || `+${event.offset_ms}ms`}
              </text>
              <text
                x={TIME_COL_W - 12}
                y={y + 10}
                fontSize={11}
                fill="hsl(var(--heroui-success))"
                fontWeight="bold"
                textAnchor="end"
              >
                {diffStr}
              </text>

              {/* Message Arrow Line (Dashed if retransmission) */}
              <line
                x1={x1}
                y1={y}
                x2={x2}
                y2={y}
                stroke={msgColor}
                strokeWidth={isRetransmission ? 1.2 : 1.5}
                strokeDasharray={isRetransmission ? '4 3' : undefined}
              />

              {/* Arrow Head */}
              {rightward ? (
                <path
                  d={`M ${x2 - 10} ${y - 4} L ${x2} ${y} L ${x2 - 10} ${y + 4}`}
                  fill="none"
                  stroke={msgColor}
                  strokeWidth={2}
                />
              ) : (
                <path
                  d={`M ${x2 + 10} ${y - 4} L ${x2} ${y} L ${x2 + 10} ${y + 4}`}
                  fill="none"
                  stroke={msgColor}
                  strokeWidth={2}
                />
              )}

              {/* Message Name Label */}
              <text
                x={(x1 + x2) / 2}
                y={y - 7}
                textAnchor="middle"
                fontSize={12}
                fontWeight="bold"
                fill={msgColor}
              >
                {event.message}
              </text>
            </g>
          );
        })}
      </svg>

      {/* Raw SIP Message Modal */}
      <Modal
        isOpen={Boolean(selectedEvent)}
        onOpenChange={(o) => !o && setSelectedEvent(null)}
        size="lg"
      >
        <ModalContent>
          <ModalHeader className="text-small font-mono text-primary">
            SIP 信令报文详情 - {selectedEvent?.message}
          </ModalHeader>
          <ModalBody>
            <div className="bg-content2 rounded-medium p-4 max-h-[500px] overflow-y-auto">
              <pre className="text-tiny font-mono whitespace-pre-wrap text-foreground">
                {selectedEvent?.raw_message}
              </pre>
              {parsedSDP && parsedSDP.length > 0 && (
                <div className="mt-4 pt-4 border-t border-divider">
                  <h4 className="text-small font-semibold text-primary mb-3">
                    SDP 媒体协商 (解析)
                  </h4>
                  {parsedSDP.map((media, i) => (
                    <div key={i} className="mb-3 p-3 rounded-medium bg-primary/10">
                      <div className="grid grid-cols-3 gap-2 text-tiny">
                        <div>
                          <span className="text-default-500">媒体类型: </span>
                          <Chip size="sm" color="primary" variant="flat">
                            {media.mediaType.toUpperCase()}
                          </Chip>
                        </div>
                        <div>
                          <span className="text-default-500">协商端口: </span>
                          <span className="font-mono text-foreground">{media.port}</span>
                        </div>
                        <div>
                          <span className="text-default-500">编解码: </span>
                          <span className="text-foreground">
                            {media.codecs.join(', ') || '未知'}
                          </span>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </ModalBody>
          <ModalFooter>
            <Button color="primary" onPress={() => setSelectedEvent(null)}>
              关闭
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </div>
  );
}

interface AuditGroupSpec {
  title: string;
  fields: Array<[keyof CdrAuditSnapshot, string]>;
}

const auditGroups: AuditGroupSpec[] = [
  {
    title: '来源信息',
    fields: [
      ['source_type', '来源类型'],
      ['source_id', '来源标识'],
      ['ingress_trunk_id', '接入中继'],
    ],
  },
  {
    title: '主叫决策',
    fields: [
      ['original_caller', '原始主叫'],
      ['presented_caller', '呈现主叫'],
      ['caller_mode', '主叫策略'],
      ['caller_pool_id', '号码池组'],
      ['caller_selection', '选号算法'],
    ],
  },
  {
    title: '落地决策',
    fields: [
      ['egress_trunk_id', '落地中继'],
      ['selected_route_id', '选中路由'],
      ['fallback_used', '发生故障切换'],
    ],
  },
  {
    title: '计费快照',
    fields: [
      ['billing_account', '计费账户'],
      ['billing_interval_secs', '计费周期'],
      ['price_per_interval', '周期价格'],
    ],
  },
];

function CallAuditSnapshot({ audit }: { audit: CdrAuditSnapshot }) {
  return (
    <Card shadow="none" className="overview-card p-2">
      <CardBody className="p-5 flex flex-col gap-4">
        <div className="flex items-center gap-2">
          <h3 className="text-base font-bold text-foreground">决策审计</h3>
          <Chip color="primary" variant="flat" size="sm">
            呼叫建立时快照
          </Chip>
        </div>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
          {auditGroups.map((group) => (
            <div key={group.title} className="flex flex-col gap-2 p-3 rounded-medium bg-content2">
              <h4 className="text-small font-semibold text-foreground">{group.title}</h4>
              {group.fields.map(([key, label]) => (
                <div key={key} className="flex flex-col">
                  <span className="text-tiny text-default-500">{label}</span>
                  <span className="text-tiny text-default-600">
                    {callDetailText(audit[key], key)}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </div>
      </CardBody>
    </Card>
  );
}

export function CallSummary({ entity }: { entity: Entity }) {
  const availability = String(entity.runtime_availability ?? 'unavailable');
  const historical =
    entity.historical && typeof entity.historical === 'object'
      ? (entity.historical as Entity)
      : null;
  const audit =
    historical?.audit && typeof historical.audit === 'object'
      ? (historical.audit as CdrAuditSnapshot)
      : null;
  const historicalSummary = historical
    ? Object.fromEntries(Object.entries(historical).filter(([key]) => key !== 'audit'))
    : null;
  const availabilityColor: 'success' | 'default' | 'warning' =
    availability === 'available'
      ? 'success'
      : availability === 'not_active'
        ? 'default'
        : 'warning';

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="overview-card p-2">
        <CardBody className="p-5 flex flex-col gap-3">
          <div className="flex items-center gap-2">
            <h3 className="text-base font-bold text-foreground">历史通话</h3>
            <Chip color={historical ? 'success' : 'default'} variant="flat" size="sm">
              {historical ? '已持久化' : '暂无 CDR'}
            </Chip>
          </div>
          <DetailFields value={historicalSummary} empty="暂无历史通话数据" />
        </CardBody>
      </Card>

      {audit && <CallAuditSnapshot audit={audit} />}

      <Card shadow="none" className="overview-card p-2">
        <CardBody className="p-5 flex flex-col gap-3">
          <div className="flex items-center gap-2">
            <h3 className="text-base font-bold text-foreground">实时状态</h3>
            <Chip color={availabilityColor} variant="flat" size="sm">
              {callDetailText(availability)}
            </Chip>
          </div>
          <DetailFields
            value={entity.runtime}
            empty={availability === 'not_active' ? '通话已结束' : '实时控制面不可用'}
          />
        </CardBody>
      </Card>
    </div>
  );
}

interface MediaMetrics {
  received_packets: number;
  dropped_packets: number;
  jitter_ms: number;
  loss_percent: number;
  rtt_ms: number;
  mos: number;
  webrtc?: {
    ice_connected: boolean;
    dtls_connected: boolean;
    dtls_failed: boolean;
  };
}

interface PlaybackInfo {
  file_path: string;
  mode: string;
  loop_playback: boolean;
  progress_percentage: number;
}

interface LegStatus {
  muted: boolean;
  playback: PlaybackInfo | null;
  metrics: MediaMetrics | null;
}

export interface CallMediaStatus {
  call_id: string;
  caller: LegStatus;
  callee: LegStatus;
  runtime_availability?: string;
}

export function CallMediaDiagnostics({ status }: { status: CallMediaStatus }) {
  const renderLeg = (title: string, leg: LegStatus) => {
    const metrics = leg.metrics;
    const playback = leg.playback;
    const mos = metrics?.mos || 0.0;
    let mosColor: 'success' | 'warning' | 'danger' = 'danger';
    let mosText = '差 (Poor)';
    if (mos >= 4.0) {
      mosColor = 'success';
      mosText = '极佳 (Excellent)';
    } else if (mos >= 3.0) {
      mosColor = 'warning';
      mosText = '中 (Fair)';
    }
    const mosTextColor =
      mosColor === 'success'
        ? 'text-success'
        : mosColor === 'warning'
          ? 'text-warning'
          : 'text-danger';

    return (
      <Card shadow="none" className="overview-card p-2">
        <CardBody className="p-5 flex flex-col gap-4">
          <h3 className="text-base font-bold text-foreground">{title}</h3>

          <div className="p-3 rounded-medium bg-content2">
            <h4 className="text-small font-semibold text-foreground mb-2">通话控制与流状态</h4>
            <div className="flex items-center gap-2 flex-wrap">
              <Chip color={leg.muted ? 'danger' : 'success'} variant="flat" size="sm">
                {leg.muted ? '静音中' : '正常收发'}
              </Chip>
              {playback && (
                <Chip color="primary" variant="flat" size="sm">
                  播放中: {playback.file_path.split('/').pop()} (
                  {playback.progress_percentage.toFixed(0)}%)
                </Chip>
              )}
            </div>
          </div>

          {metrics ? (
            <div className="flex flex-col gap-3">
              <h4 className="text-small font-semibold text-foreground border-b border-divider pb-2">
                媒体层质量 (RTP/RTCP)
              </h4>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <div className="flex flex-col items-center gap-2 p-3 rounded-medium bg-content2">
                  <span className="text-tiny text-default-500">通话质量评分</span>
                  <Progress
                    size="lg"
                    value={Math.min(100, Math.max(0, (mos / 5.0) * 100))}
                    color={mosColor}
                    showValueLabel
                    aria-label="通话质量"
                  />
                  <span className={`text-tiny font-bold ${mosTextColor}`}>
                    {mos.toFixed(2)} · {mosText}
                  </span>
                </div>
                <div className="md:col-span-2 grid grid-cols-1 sm:grid-cols-2 gap-2">
                  <div className="flex justify-between p-2 rounded-medium bg-content2">
                    <span className="text-tiny text-default-500">均方抖动</span>
                    <span className="text-tiny font-mono text-foreground">
                      {metrics.jitter_ms.toFixed(2)} ms
                    </span>
                  </div>
                  <div className="flex justify-between p-2 rounded-medium bg-content2">
                    <span className="text-tiny text-default-500">均方延时</span>
                    <span className="text-tiny font-mono text-foreground">{metrics.rtt_ms} ms</span>
                  </div>
                  <div className="flex justify-between p-2 rounded-medium bg-content2">
                    <span className="text-tiny text-default-500">接收数据包</span>
                    <span className="text-tiny font-mono text-foreground">
                      {metrics.received_packets} 包
                    </span>
                  </div>
                  <div className="flex justify-between p-2 rounded-medium bg-content2">
                    <span className="text-tiny text-default-500">丢包率</span>
                    <span
                      className={`text-tiny font-mono ${metrics.loss_percent > 2.0 ? 'text-danger' : 'text-foreground'}`}
                    >
                      {metrics.loss_percent.toFixed(2)} %
                    </span>
                  </div>
                  <div className="flex justify-between p-2 rounded-medium bg-content2 sm:col-span-2">
                    <span className="text-tiny text-default-500">失效包丢弃</span>
                    <span className="text-tiny font-mono text-foreground">
                      {metrics.dropped_packets} 包
                    </span>
                  </div>
                </div>
              </div>

              {metrics.webrtc &&
                (metrics.webrtc.ice_connected || metrics.webrtc.dtls_connected) && (
                  <div className="flex flex-col gap-2 pt-2 border-t border-divider">
                    <h4 className="text-small font-semibold text-foreground">WebRTC 握手状态</h4>
                    <div className="flex items-center gap-2 flex-wrap">
                      <Chip
                        color={metrics.webrtc.ice_connected ? 'success' : 'default'}
                        variant="flat"
                        size="sm"
                      >
                        ICE: {metrics.webrtc.ice_connected ? '已连接' : '未连接'}
                      </Chip>
                      <Chip
                        color={
                          metrics.webrtc.dtls_failed
                            ? 'danger'
                            : metrics.webrtc.dtls_connected
                              ? 'success'
                              : 'default'
                        }
                        variant="flat"
                        size="sm"
                      >
                        DTLS:{' '}
                        {metrics.webrtc.dtls_failed
                          ? '失败'
                          : metrics.webrtc.dtls_connected
                            ? '已加密'
                            : '未建立'}
                      </Chip>
                    </div>
                  </div>
                )}
            </div>
          ) : (
            <div className="py-6 text-center text-small text-default-400">
              暂无实时媒体质量统计（通道尚未激活包收发）
            </div>
          )}
        </CardBody>
      </Card>
    );
  };

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4 py-2">
      {renderLeg('主叫侧媒体通道 (Caller Leg)', status.caller)}
      {renderLeg('被叫侧媒体通道 (Callee Leg)', status.callee)}
    </div>
  );
}
