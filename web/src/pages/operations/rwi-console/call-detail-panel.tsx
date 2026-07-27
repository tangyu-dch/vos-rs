import { Card, CardBody, Chip } from '@heroui/react';
import { Bot, Sparkles, User, Waves } from 'lucide-react';
import type { CallState, LiveCallItem } from './use-rwi-websocket';
import { AudioWaveform } from './audio-waveform';
import { AiAgentPanel } from './ai-agent-panel';

interface CallDetailPanelProps {
  currentCall: LiveCallItem;
  isOperatorOrAdmin: boolean;
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
  isOperatorOrAdmin,
  transcriptScrollRef,
  renderStateChip,
  onBargeIn,
  onSpeak,
  onToggleListen,
  onTransfer,
  onHangup,
}: CallDetailPanelProps) {
  const aiStatusText =
    currentCall.aiStatus === 'speaking' ? '🗣️ AI正在播报发言'
    : currentCall.aiStatus === 'listening' ? '👂 AI正在倾听用户'
    : currentCall.aiStatus === 'thinking' ? '🧠 AI大模型推理中...'
    : currentCall.aiStatus === 'barge_in' ? '⚡ 已被坐席抢断接管'
    : '💤 待命';

  return (
    <Card shadow="sm" className="bg-content1/80 border border-default-200/60 backdrop-blur-md flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Header Panel for Selected Call */}
      <div className="p-4 bg-content2/80 border-b border-default-200/60 flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-primary/10 text-primary border border-primary/20 flex items-center justify-center">
            <Bot className="w-5 h-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-bold text-foreground font-mono">{currentCall.callId}</h2>
              {renderStateChip(currentCall.state)}
              {currentCall.listening && (
                <Chip size="sm" color="success" variant="flat" className="animate-pulse">
                  🎧 实时监听中
                </Chip>
              )}
            </div>
            <div className="text-xs text-default-500 mt-0.5 flex items-center gap-3">
              <span>主叫: <strong className="text-foreground font-mono">{currentCall.caller}</strong></span>
              <span>{"->"}</span>
              <span>被叫: <strong className="text-foreground font-mono">{currentCall.callee}</strong></span>
              <span>•</span>
              <span>中继: <span className="font-mono">{currentCall.gateway}</span></span>
            </div>
          </div>
        </div>

        {/* AI Agent Mode Indicator */}
        <div className="flex items-center gap-2">
          <div className="px-3 py-1.5 rounded-lg bg-primary/5 border border-primary/10 flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-primary animate-spin" />
            <span className="text-xs font-semibold text-primary">{aiStatusText}</span>
          </div>
        </div>
      </div>

      <CardBody className="p-4 flex flex-col gap-4 flex-1 min-h-0 overflow-y-auto">
        {/* Interactive AI Agent Control Action Bar */}
        <AiAgentPanel
          currentCall={currentCall}
          isOperatorOrAdmin={isOperatorOrAdmin}
          onBargeIn={onBargeIn}
          onSpeak={onSpeak}
          onToggleListen={onToggleListen}
          onTransfer={onTransfer}
          onHangup={onHangup}
        />

        {/* Media Stream Realtime Spectrum & Metrics Panel */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {/* Inbound Stream Metrics (Caller -> System) */}
          <div className="p-3.5 rounded-xl bg-content2/70 border border-default-200/50 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-xs font-bold text-foreground flex items-center gap-1.5">
                <User className="w-3.5 h-3.5 text-primary" />
                主叫上行 RTP 媒体流 (Inbound)
              </span>
              <span className="font-mono text-tiny text-primary">{currentCall.media.codec}</span>
            </div>

            <div className="flex items-center justify-between gap-3">
              <AudioWaveform
                active={currentCall.media.audioLevelIn > 5}
                level={currentCall.media.audioLevelIn}
                color="primary"
              />
              <div className="text-right font-mono text-tiny text-default-400 space-y-0.5">
                <div>Bitrate: <span className="text-foreground">{currentCall.media.bitrateKbps} kbps</span></div>
                <div>Loss: <span className="text-success">{currentCall.media.packetLossPercent}%</span></div>
              </div>
            </div>
          </div>

          {/* Outbound Stream Metrics (AI Agent -> Caller) */}
          <div className="p-3.5 rounded-xl bg-content2/70 border border-default-200/50 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-xs font-bold text-foreground flex items-center gap-1.5">
                <Bot className="w-3.5 h-3.5 text-primary" />
                AI 下行 TTS 媒体流 (Outbound)
              </span>
              <span className="font-mono text-tiny text-primary">Opus/48k (Low Latency)</span>
            </div>

            <div className="flex items-center justify-between gap-3">
              <AudioWaveform
                active={currentCall.media.audioLevelOut > 5}
                level={currentCall.media.audioLevelOut}
                color="primary"
              />
              <div className="text-right font-mono text-tiny text-default-400 space-y-0.5">
                <div>Jitter: <span className="text-foreground">{currentCall.media.jitterMs} ms</span></div>
                <div>RTT: <span className="text-primary">{currentCall.media.rttMs} ms</span></div>
              </div>
            </div>
          </div>
        </div>

        {/* Live ASR Subtitle & Transcript Stream */}
        <div className="flex-1 flex flex-col min-h-[280px] bg-content2 rounded-xl border border-default-200/50 p-4">
          <div className="flex items-center justify-between pb-3 mb-3 border-b border-default-200">
            <div className="flex items-center gap-2">
              <Waves className="w-4 h-4 text-primary animate-pulse" />
              <h4 className="text-xs font-bold text-foreground">实时 ASR 语音识别字幕与对话流</h4>
            </div>
            <div className="text-[10px] text-default-400 font-mono">
              WebSocket Event: <span className="text-success">asr_stream_active</span>
            </div>
          </div>

          {/* Transcript Scroll Log */}
          <div ref={transcriptScrollRef} className="flex-1 overflow-y-auto space-y-3 pr-1">
            {currentCall.transcripts.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-default-400 text-xs gap-2">
                <Bot className="w-8 h-8 opacity-30" />
                <p>等待语音流建立与 ASR 文本输出...</p>
              </div>
            ) : (
              currentCall.transcripts.map((t) => {
                const isUser = t.speaker === 'user';
                const isSystem = t.speaker === 'system';

                if (isSystem) {
                  return (
                    <div key={t.id} className="flex justify-center my-2">
                      <span className={`text-[11px] font-mono px-3 py-1 rounded-full border ${
                        t.interrupted
                          ? 'bg-danger/20 text-danger border-danger/30'
                          : 'bg-primary/20 text-primary border-primary/30'
                      }`}>
                        {t.text} ({t.timestamp})
                      </span>
                    </div>
                  );
                }

                return (
                  <div
                    key={t.id}
                    className={`flex gap-3 max-w-[85%] ${isUser ? 'mr-auto' : 'ml-auto flex-row-reverse'}`}
                  >
                    <div className={`w-8 h-8 rounded-full flex items-center justify-center shrink-0 font-bold text-xs ${
                      isUser
                        ? 'bg-primary/10 text-primary border border-primary/30'
                        : 'bg-primary/20 text-primary border border-primary/30'
                    }`}>
                      {isUser ? <User className="w-4 h-4" /> : <Bot className="w-4 h-4" />}
                    </div>

                    <div>
                      <div className={`flex items-center gap-2 mb-1 text-[10px] text-default-400 ${isUser ? '' : 'flex-row-reverse'}`}>
                        <span className="font-semibold">{isUser ? '用户 (Caller)' : currentCall.aiAgentName}</span>
                        <span>•</span>
                        <span className="font-mono">{t.timestamp}</span>
                        {t.latencyMs && (
                          <span className="text-success font-mono">({t.latencyMs}ms)</span>
                        )}
                      </div>

                      <div className={`p-3 rounded-2xl text-xs leading-relaxed ${
                        isUser
                          ? 'bg-default-100 text-foreground rounded-tl-none border border-default-200/60'
                          : 'bg-primary/10 text-foreground rounded-tr-none border border-primary/30 shadow-lg shadow-primary/10'
                      }`}>
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
