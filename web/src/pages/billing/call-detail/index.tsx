// 通话记录详情视图
// 展示 CDR 历史数据 + 运行时状态（若通话仍在进行）

import { useCallback, useEffect, useState } from 'react';
import { Card, CardBody, Chip, Tabs, Tab } from '@heroui/react';
import { PhoneCall, Radio, Activity, FileAudio, Route } from 'lucide-react';
import { api } from '@/services/client';
import { DetailErrorState, DetailLoading, SectionBlock } from '@/components/detail-shell';
import { RecordingPlayer } from '@/components/recording-player';
import { CallSipFlowTab } from './call-sipflow-tab';
import {
  callDetailText,
  datetimeText,
  durationSecondsText,
  hangupCauseText,
  moneyText,
  valueText,
} from '@/pages/shared/format';

interface CallDetailProps {
  /** 通话 ID */
  id: string;
}

interface RuntimeState {
  state?: string;
  caller?: string;
  callee?: string;
  gateway?: string;
  direction?: string;
  muted?: boolean;
  playback?: unknown;
  runtime_availability?: string;
}

interface HistoricalCdr {
  call_id?: string;
  caller?: string;
  callee?: string;
  direction?: string;
  status?: string;
  started_at_ms?: number;
  ringing_at_ms?: number | null;
  answered_at_ms?: number | null;
  ended_at_ms?: number | null;
  duration_ms?: number;
  billable_duration_ms?: number;
  talk_duration_ms?: number | null;
  ringing_duration_ms?: number | null;
  access_billable_duration_ms?: number | null;
  access_charge_amount?: number | null;
  egress_billable_duration_ms?: number | null;
  egress_cost_amount?: number | null;
  failure_status_code?: number | null;
  failure_reason?: string | null;
  caller_rtcp_loss_rate?: number | null;
  caller_rtcp_jitter_ms?: number | null;
  caller_rtcp_rtt_ms?: number | null;
  gateway_rtcp_loss_rate?: number | null;
  gateway_rtcp_jitter_ms?: number | null;
  gateway_rtcp_rtt_ms?: number | null;
  mos?: number | null;
  dtmf_digits?: string | null;
  recording_path?: string | null;
  audit?: {
    source_type?: string | null;
    source_id?: string | null;
    billing_account?: string | null;
    original_caller?: string | null;
    presented_caller?: string | null;
    caller_mode?: string | null;
    caller_pool_id?: string | null;
    caller_selection?: string | null;
    ingress_trunk_id?: string | null;
    egress_trunk_id?: string | null;
    selected_route_id?: string | null;
    fallback_used?: boolean;
    billing_interval_secs?: number | null;
    price_per_interval?: number | null;
    egress_billing_account?: string | null;
    egress_billing_interval_secs?: number | null;
    egress_price_per_interval?: number | null;
  } | null;
}

interface CallDetailResponse {
  historical: HistoricalCdr | null;
  runtime: RuntimeState | null;
  runtime_availability: string;
}

// 媒体质量评估等级
function mosLevel(mos: number | null | undefined): {
  color: 'success' | 'warning' | 'danger';
  label: string;
} {
  if (mos === null || mos === undefined) return { color: 'default' as never, label: '未评估' };
  if (mos >= 4.0) return { color: 'success', label: '优秀' };
  if (mos >= 3.5) return { color: 'warning', label: '良好' };
  return { color: 'danger', label: '较差' };
}

// 通话状态颜色映射
function statusColor(status: string | undefined): 'success' | 'danger' | 'warning' | 'default' {
  switch (status) {
    case 'answered':
      return 'success';
    case 'failed':
      return 'danger';
    case 'canceled':
      return 'warning';
    default:
      return 'default';
  }
}

// 运行时可用性标签
function runtimeLabel(avail: string | undefined): {
  color: 'success' | 'warning' | 'default';
  text: string;
} {
  switch (avail) {
    case 'available':
      return { color: 'success', text: '通话进行中' };
    case 'not_active':
      return { color: 'default', text: '通话已结束' };
    case 'unavailable':
      return { color: 'warning', text: '控制面暂不可用' };
    default:
      return { color: 'default', text: '未知' };
  }
}

function sipIdentityText(value: unknown): string {
  const text = valueText(value);
  if (text === '—') return text;
  const sipUser = text.match(/sip:([^@;>]+)/i)?.[1];
  return sipUser || text.match(/^"([^"]+)"/)?.[1] || text;
}

// 键值展示行
function DetailRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: unknown;
  mono?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-3 py-1.5 border-b border-divider/40 last:border-0">
      <span className="text-tiny text-default-400 flex-shrink-0 pt-0.5">{label}</span>
      <span className={`text-small text-foreground text-right ${mono ? 'font-mono' : ''}`}>
        {callDetailText(value, label)}
      </span>
    </div>
  );
}

export function CallDetailView({ id }: CallDetailProps) {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [detail, setDetail] = useState<CallDetailResponse | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const payload = await api.get<CallDetailResponse>(`/calls/${encodeURIComponent(id)}`);
      setDetail(payload);
    } catch (e) {
      const msg = e instanceof Error ? e.message : '加载通话详情失败';
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    load();
  }, [load]);

  if (loading) return <DetailLoading />;
  if (error) return <DetailErrorState error={error} />;
  if (!detail) return <DetailErrorState error="未获取到通话详情数据" />;

  const cdr = detail.historical;
  const runtime = detail.runtime;
  const rt = runtimeLabel(detail.runtime_availability);
  const mosInfo = cdr ? mosLevel(cdr.mos) : { color: 'default' as never, label: '未评估' };
  const ringingDuration =
    cdr?.ringing_duration_ms ??
    (cdr?.ringing_at_ms && (cdr.answered_at_ms || cdr.ended_at_ms)
      ? Math.max(0, (cdr.answered_at_ms || cdr.ended_at_ms || 0) - cdr.ringing_at_ms)
      : null);

  return (
    <div className="flex flex-col gap-4">
      {/* 顶部摘要 */}
      <Card className="border border-default-200 bg-content1">
        <CardBody className="p-4">
          <div className="flex items-center justify-between flex-wrap gap-3">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center">
                <PhoneCall className="w-5 h-5 text-primary" />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <span className="text-small font-bold text-foreground font-mono">{id}</span>
                  <Chip size="sm" variant="flat" color={rt.color}>
                    {rt.text}
                  </Chip>
                </div>
                <div className="text-tiny text-default-400 mt-0.5">
                  {sipIdentityText(cdr?.caller)} → {sipIdentityText(cdr?.callee)}
                </div>
              </div>
            </div>
            <div className="flex items-center gap-2">
              {cdr?.status && (
                <Chip size="sm" variant="flat" color={statusColor(cdr.status)}>
                  {callDetailText(cdr.status, 'status')}
                </Chip>
              )}
              <Chip size="sm" variant="flat" color={mosInfo.color}>
                质量 {mosInfo.label}
              </Chip>
            </div>
          </div>
        </CardBody>
      </Card>

      {/* 分标签展示 */}
      <Tabs aria-label="通话详情标签" variant="underlined">
        {/* 基本信息标签 */}
        <Tab
          key="basic"
          title={
            <div className="flex items-center gap-1.5">
              <PhoneCall className="w-3.5 h-3.5" />
              <span>基本信息</span>
            </div>
          }
        >
          <div className="flex flex-col gap-4">
            <Card className="border border-default-200">
              <CardBody className="p-4">
                {cdr ? (
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-1">
                    <DetailRow label="主叫号码" value={sipIdentityText(cdr.caller)} />
                    <DetailRow label="被叫号码" value={sipIdentityText(cdr.callee)} />
                    <DetailRow label="呼叫方向" value={cdr.direction} />
                    <DetailRow label="通话状态" value={cdr.status} />
                    <DetailRow
                      label="挂断原因 (Hangup Cause)"
                      value={hangupCauseText(cdr.failure_status_code, cdr.failure_reason)}
                    />
                    <DetailRow
                      label="总时长"
                      value={cdr.duration_ms ? `${durationSecondsText(cdr.duration_ms)} 秒` : '—'}
                    />
                    {ringingDuration !== null && ringingDuration !== undefined && (
                      <DetailRow
                        label="振铃时长"
                        value={`${durationSecondsText(ringingDuration)} 秒`}
                      />
                    )}
                    {cdr.talk_duration_ms !== null && cdr.talk_duration_ms !== undefined && (
                      <DetailRow
                        label="通话时长"
                        value={`${durationSecondsText(cdr.talk_duration_ms)} 秒`}
                      />
                    )}
                    {cdr.billable_duration_ms !== null &&
                      cdr.billable_duration_ms !== undefined && (
                        <DetailRow
                          label="计费时长"
                          value={`${durationSecondsText(cdr.billable_duration_ms)} 秒`}
                        />
                      )}
                    <DetailRow
                      label="开始时间"
                      value={cdr.started_at_ms ? datetimeText(cdr.started_at_ms) : '—'}
                    />
                    {cdr.ringing_at_ms && (
                      <DetailRow label="振铃时间" value={datetimeText(cdr.ringing_at_ms)} />
                    )}
                    <DetailRow
                      label="接通时间"
                      value={cdr.answered_at_ms ? datetimeText(cdr.answered_at_ms) : '—'}
                    />
                    <DetailRow
                      label="结束时间"
                      value={cdr.ended_at_ms ? datetimeText(cdr.ended_at_ms) : '—'}
                    />
                    {cdr.access_billable_duration_ms !== null &&
                      cdr.access_billable_duration_ms !== undefined && (
                        <DetailRow
                          label="对接计费时长"
                          value={`${durationSecondsText(cdr.access_billable_duration_ms)} 秒`}
                        />
                      )}
                    {cdr.access_charge_amount !== null &&
                      cdr.access_charge_amount !== undefined && (
                        <DetailRow
                          label="对接费用"
                          value={`${moneyText(cdr.access_charge_amount)} 元`}
                        />
                      )}
                    {cdr.egress_billable_duration_ms !== null &&
                      cdr.egress_billable_duration_ms !== undefined && (
                        <DetailRow
                          label="落地计费时长"
                          value={`${durationSecondsText(cdr.egress_billable_duration_ms)} 秒`}
                        />
                      )}
                    {cdr.egress_cost_amount !== null && cdr.egress_cost_amount !== undefined && (
                      <DetailRow
                        label="成本费用"
                        value={`${moneyText(cdr.egress_cost_amount)} 元`}
                      />
                    )}
                  </div>
                ) : (
                  <p className="text-small text-default-400">无历史 CDR 数据</p>
                )}
              </CardBody>
            </Card>
            {cdr?.audit && (
              <SectionBlock title="路由与计费" description="呼叫建立时的路由与计费决策快照">
                <Card className="border border-default-200">
                  <CardBody className="p-4">
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-1">
                      {cdr.audit.billing_account && (
                        <DetailRow label="对接账户" value={cdr.audit.billing_account} mono />
                      )}
                      {cdr.audit.billing_interval_secs !== null &&
                        cdr.audit.billing_interval_secs !== undefined && (
                          <DetailRow
                            label="对接计费周期"
                            value={`${cdr.audit.billing_interval_secs} 秒`}
                          />
                        )}
                      {cdr.audit.price_per_interval !== null &&
                        cdr.audit.price_per_interval !== undefined && (
                          <DetailRow
                            label="对接周期价格"
                            value={`${moneyText(cdr.audit.price_per_interval)} 元`}
                          />
                        )}
                      {cdr.audit.egress_billing_account && (
                        <DetailRow label="落地账户" value={cdr.audit.egress_billing_account} mono />
                      )}
                      {cdr.audit.egress_billing_interval_secs !== null &&
                        cdr.audit.egress_billing_interval_secs !== undefined && (
                          <DetailRow
                            label="落地计费周期"
                            value={`${cdr.audit.egress_billing_interval_secs} 秒`}
                          />
                        )}
                      {cdr.audit.egress_price_per_interval !== null &&
                        cdr.audit.egress_price_per_interval !== undefined && (
                          <DetailRow
                            label="落地周期价格"
                            value={`${moneyText(cdr.audit.egress_price_per_interval)} 元`}
                          />
                        )}
                      {cdr.audit.ingress_trunk_id && (
                        <DetailRow label="对接中继" value={cdr.audit.ingress_trunk_id} mono />
                      )}
                      {cdr.audit.egress_trunk_id && (
                        <DetailRow label="落地中继" value={cdr.audit.egress_trunk_id} mono />
                      )}
                      {cdr.audit.selected_route_id && (
                        <DetailRow label="选中路由" value={cdr.audit.selected_route_id} mono />
                      )}
                      {cdr.audit.original_caller && (
                        <DetailRow
                          label="原始主叫"
                          value={sipIdentityText(cdr.audit.original_caller)}
                        />
                      )}
                      {cdr.audit.presented_caller && (
                        <DetailRow
                          label="显示主叫"
                          value={sipIdentityText(cdr.audit.presented_caller)}
                        />
                      )}
                      {cdr.audit.caller_mode && (
                        <DetailRow label="主叫模式" value={cdr.audit.caller_mode} />
                      )}
                      {cdr.audit.caller_pool_id && (
                        <DetailRow label="主叫号码池" value={cdr.audit.caller_pool_id} mono />
                      )}
                      {cdr.audit.caller_selection && (
                        <DetailRow label="选号策略" value={cdr.audit.caller_selection} />
                      )}
                      {cdr.audit.source_type && (
                        <DetailRow label="路由来源类型" value={cdr.audit.source_type} />
                      )}
                      {cdr.audit.source_id && (
                        <DetailRow label="路由来源标识" value={cdr.audit.source_id} mono />
                      )}
                      <DetailRow
                        label="是否故障切换"
                        value={cdr.audit.fallback_used ? '是' : '否'}
                      />
                    </div>
                  </CardBody>
                </Card>
              </SectionBlock>
            )}
          </div>
        </Tab>

        <Tab
          key="signaling"
          title={
            <div className="flex items-center gap-1.5">
              <Route className="w-3.5 h-3.5" />
              <span>信令流程</span>
            </div>
          }
        >
          <CallSipFlowTab id={id} />
        </Tab>

        {/* 媒体质量标签 */}
        <Tab
          key="media"
          title={
            <div className="flex items-center gap-1.5">
              <Activity className="w-3.5 h-3.5" />
              <span>媒体质量</span>
            </div>
          }
        >
          <Card className="border border-default-200">
            <CardBody className="p-4">
              {cdr &&
              (cdr.caller_rtcp_loss_rate !== null || cdr.gateway_rtcp_loss_rate !== null) ? (
                <div className="grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-1">
                  <DetailRow
                    label="主叫丢包率"
                    value={
                      cdr.caller_rtcp_loss_rate !== null
                        ? `${moneyText(Number(cdr.caller_rtcp_loss_rate) * 100)}%`
                        : '—'
                    }
                  />
                  <DetailRow
                    label="落地丢包率"
                    value={
                      cdr.gateway_rtcp_loss_rate !== null
                        ? `${moneyText(Number(cdr.gateway_rtcp_loss_rate) * 100)}%`
                        : '—'
                    }
                  />
                  <DetailRow
                    label="主叫抖动"
                    value={
                      cdr.caller_rtcp_jitter_ms !== null
                        ? `${moneyText(cdr.caller_rtcp_jitter_ms)} 毫秒`
                        : '—'
                    }
                  />
                  <DetailRow
                    label="落地抖动"
                    value={
                      cdr.gateway_rtcp_jitter_ms !== null
                        ? `${moneyText(cdr.gateway_rtcp_jitter_ms)} 毫秒`
                        : '—'
                    }
                  />
                  <DetailRow
                    label="主叫往返时延"
                    value={
                      cdr.caller_rtcp_rtt_ms !== null
                        ? `${moneyText(cdr.caller_rtcp_rtt_ms)} 毫秒`
                        : '—'
                    }
                  />
                  <DetailRow
                    label="落地往返时延"
                    value={
                      cdr.gateway_rtcp_rtt_ms !== null
                        ? `${moneyText(cdr.gateway_rtcp_rtt_ms)} 毫秒`
                        : '—'
                    }
                  />
                  <DetailRow label="通话质量评分" value={cdr.mos} />
                </div>
              ) : (
                <p className="text-small text-default-400">无媒体质量数据</p>
              )}
            </CardBody>
          </Card>
        </Tab>

        {/* 实时状态标签（仅通话进行中时有数据） */}
        <Tab
          key="runtime"
          title={
            <div className="flex items-center gap-1.5">
              <Radio className="w-3.5 h-3.5" />
              <span>实时状态</span>
            </div>
          }
        >
          <Card className="border border-default-200">
            <CardBody className="p-4">
              {runtime ? (
                <div className="grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-1">
                  <DetailRow label="实时状态" value={runtime.state} />
                  <DetailRow label="运行可用性" value={detail.runtime_availability} />
                  <DetailRow label="主叫号码" value={runtime.caller} />
                  <DetailRow label="被叫号码" value={runtime.callee} />
                  <DetailRow label="当前中继" value={runtime.gateway} mono />
                  <DetailRow label="静音状态" value={runtime.muted} />
                </div>
              ) : (
                <p className="text-small text-default-400">
                  {detail.runtime_availability === 'not_active'
                    ? '通话已结束，无实时状态'
                    : '实时状态不可用'}
                </p>
              )}
            </CardBody>
          </Card>
        </Tab>

        {/* 录音与 DTMF 标签 */}
        <Tab
          key="recording"
          title={
            <div className="flex items-center gap-1.5">
              <FileAudio className="w-3.5 h-3.5" />
              <span>录音按键</span>
            </div>
          }
        >
          <div className="flex flex-col gap-3">
            <SectionBlock title="录音文件" description="通话录音的存储路径与回放">
              <Card className="border border-default-200">
                <CardBody className="p-4">
                  {cdr?.recording_path ? (
                    <div className="flex items-center gap-3">
                      <FileAudio className="w-5 h-5 text-primary flex-shrink-0" />
                      <div className="flex-1 min-w-0">
                        <p className="text-small text-foreground font-mono truncate">
                          {valueText(cdr.recording_path)}
                        </p>
                        <RecordingPlayer
                          className="mt-2 w-full h-8"
                          source={`/calls/${encodeURIComponent(id)}/recording`}
                        />
                      </div>
                    </div>
                  ) : (
                    <p className="text-small text-default-400">本通话未启用录音或无录音文件</p>
                  )}
                </CardBody>
              </Card>
            </SectionBlock>

            <SectionBlock title="按键记录" description="通话过程中的按键序列">
              <Card className="border border-default-200">
                <CardBody className="p-4">
                  {cdr?.dtmf_digits ? (
                    <div className="flex items-center gap-2">
                      <span className="text-tiny text-default-400">按键</span>
                      <span className="text-small font-mono text-foreground bg-content2 px-2 py-1 rounded">
                        {valueText(cdr.dtmf_digits)}
                      </span>
                    </div>
                  ) : (
                    <p className="text-small text-default-400">无按键记录</p>
                  )}
                </CardBody>
              </Card>
            </SectionBlock>

            {cdr?.failure_reason && (
              <SectionBlock title="失败诊断" description="通话失败时的原因与状态码">
                <Card className="border border-danger/30 bg-danger/5">
                  <CardBody className="p-4">
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-x-6 gap-y-1">
                      <DetailRow label="失败状态码" value={cdr.failure_status_code} mono />
                      <DetailRow label="失败原因" value={cdr.failure_reason} />
                    </div>
                  </CardBody>
                </Card>
              </SectionBlock>
            )}
          </div>
        </Tab>
      </Tabs>
    </div>
  );
}
