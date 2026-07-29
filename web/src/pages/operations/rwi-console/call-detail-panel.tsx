import { Card, CardBody, Chip } from '@heroui/react';
import { Phone, User, Waves } from 'lucide-react';
import type { CallState, LiveCallItem } from './use-rwi-websocket';
import { AudioWaveform } from './audio-waveform';
import { CallControlPanel } from './ai-agent-panel';

interface CallDetailPanelProps {
  currentCall: LiveCallItem;
  permissions: {
    canBarge: boolean;
    canPlay: boolean;
    canMonitor: boolean;
    canTransfer: boolean;
    canTerminate: boolean;
  };
  transcriptScrollRef: React.RefObject<HTMLDivElement>;
  renderStateChip: (state: CallState) => React.ReactNode;
  onBargeIn: () => void;
  onSpeak: () => void;
  onToggleListen: () => void;
  onTransfer: () => void;
  onHangup: () => void;
}

export function CallDetailPanel({
  currentCall,
  permissions,
  transcriptScrollRef,
  renderStateChip,
  onBargeIn,
  onSpeak,
  onToggleListen,
  onTransfer,
  onHangup,
}: CallDetailPanelProps) {
  // 媒体统计字段（后端可能仅推送部分，使用默认值兜底）
  const codec = currentCall.media.codec ?? 'PCM';
  const bitrateKbps = currentCall.media.bitrateKbps ?? 0;
  const packetLossPercent = currentCall.media.packetLossPercent ?? 0;
  const jitterMs = currentCall.media.jitterMs ?? 0;
  const rttMs = currentCall.media.rttMs ?? 0;
  const audioIn = currentCall.media.audioLevelIn ?? 0;
  const audioOut = currentCall.media.audioLevelOut ?? 0;

  return (
    <Card shadow="none" className="overview-card flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* 通话信息头部 */}
      <div className="p-4 bg-content2/80 border-b border-default-200/60 flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-primary/10 text-primary border border-primary/20 flex items-center justify-center">
            <Phone className="w-5 h-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-bold text-foreground font-mono">
                {currentCall.callId}
              </h2>
              {renderStateChip(currentCall.state)}
              {currentCall.listening && (
                <Chip size="sm" color="success" variant="flat" className="animate-pulse">
                  实时监听中
                </Chip>
              )}
            </div>
            <div className="text-xs text-default-500 mt-0.5 flex items-center gap-3 flex-wrap">
              <span>
                主叫:{' '}
                <strong className="text-foreground font-mono">{currentCall.caller || '-'}</strong>
              </span>
              <span>{'->'}</span>
              <span>
                被叫:{' '}
                <strong className="text-foreground font-mono">{currentCall.callee || '-'}</strong>
              </span>
              <span>•</span>
              <span>
                方向:{' '}
                <span className="font-mono">
                  {currentCall.direction === 'inbound' ? '入呼' : '外呼'}
                </span>
              </span>
              <span>•</span>
              <span>
                网关: <span className="font-mono">{currentCall.gateway || '-'}</span>
              </span>
            </div>
          </div>
        </div>
      </div>

      <CardBody className="p-4 flex flex-col gap-4 flex-1 min-h-0 overflow-y-auto">
        {/* 通话操作面板 */}
        <CallControlPanel
          currentCall={currentCall}
          permissions={permissions}
          onBargeIn={onBargeIn}
          onSpeak={onSpeak}
          onToggleListen={onToggleListen}
          onTransfer={onTransfer}
          onHangup={onHangup}
        />

        {/* 媒体流实时统计 */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {/* 主叫上行 Inbound */}
          <div className="p-3.5 rounded-xl bg-content2/70 border border-default-200/50 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-xs font-bold text-foreground flex items-center gap-1.5">
                <User className="w-3.5 h-3.5 text-primary" />
                主叫上行媒体流
              </span>
              <span className="font-mono text-tiny text-primary">{codec}</span>
            </div>

            <div className="flex items-center justify-between gap-3">
              <AudioWaveform active={audioIn > 5} level={audioIn} color="primary" />
              <div className="text-right font-mono text-tiny text-default-400 space-y-0.5">
                <div>
                  Bitrate: <span className="text-foreground">{bitrateKbps} kbps</span>
                </div>
                <div>
                  Loss: <span className="text-success">{packetLossPercent}%</span>
                </div>
              </div>
            </div>
          </div>

          {/* 系统下行 Outbound */}
          <div className="p-3.5 rounded-xl bg-content2/70 border border-default-200/50 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-xs font-bold text-foreground flex items-center gap-1.5">
                <Phone className="w-3.5 h-3.5 text-primary" />
                系统下行媒体流 (Outbound)
              </span>
              <span className="font-mono text-tiny text-primary">{codec}</span>
            </div>

            <div className="flex items-center justify-between gap-3">
              <AudioWaveform active={audioOut > 5} level={audioOut} color="primary" />
              <div className="text-right font-mono text-tiny text-default-400 space-y-0.5">
                <div>
                  Jitter: <span className="text-foreground">{jitterMs} ms</span>
                </div>
                <div>
                  RTT: <span className="text-primary">{rttMs} ms</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* 事件 / DTMF / 系统消息转写流 */}
        <div className="flex-1 flex flex-col min-h-[280px] bg-content2 rounded-xl border border-default-200/50 p-4">
          <div className="flex items-center justify-between pb-3 mb-3 border-b border-default-200">
            <div className="flex items-center gap-2">
              <Waves className="w-4 h-4 text-primary animate-pulse" />
              <h4 className="text-xs font-bold text-foreground">事件流 / 系统消息</h4>
            </div>
            <div className="text-[10px] text-default-400 font-mono">
              实时事件: <span className="text-success">{currentCall.state}</span>
            </div>
          </div>

          {/* 滚动日志 */}
          <div ref={transcriptScrollRef} className="flex-1 overflow-y-auto space-y-3 pr-1">
            {currentCall.transcripts.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-default-400 text-xs gap-2">
                <Waves className="w-8 h-8 opacity-30" />
                <p>等待事件流推送…</p>
              </div>
            ) : (
              currentCall.transcripts.map((t) => {
                if (t.speaker === 'system') {
                  return (
                    <div key={t.id} className="flex justify-center my-2">
                      <span
                        className={`text-[11px] font-mono px-3 py-1 rounded-full border ${
                          t.interrupted
                            ? 'bg-danger/20 text-danger border-danger/30'
                            : 'bg-primary/20 text-primary border-primary/30'
                        }`}
                      >
                        {t.text} ({t.timestamp})
                      </span>
                    </div>
                  );
                }
                // user 消息样式
                return (
                  <div key={t.id} className="flex gap-3 max-w-[85%] mr-auto">
                    <div className="w-8 h-8 rounded-full flex items-center justify-center shrink-0 font-bold text-xs bg-primary/10 text-primary border border-primary/30">
                      <User className="w-4 h-4" />
                    </div>
                    <div>
                      <div className="flex items-center gap-2 mb-1 text-[10px] text-default-400">
                        <span className="font-semibold">用户 (Caller)</span>
                        <span>•</span>
                        <span className="font-mono">{t.timestamp}</span>
                        {t.latencyMs && (
                          <span className="text-success font-mono">({t.latencyMs}ms)</span>
                        )}
                      </div>
                      <div className="p-3 rounded-2xl text-xs leading-relaxed bg-default-100 text-foreground rounded-tl-none border border-default-200/60">
                        {t.text}
                      </div>
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </div>
      </CardBody>
    </Card>
  );
}
