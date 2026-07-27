import { useEffect, useRef, useState } from 'react';
import { Button, Card, Chip } from '@heroui/react';
import { Radio, PhoneCall, Sparkles, RefreshCw, Bot, Zap, Activity } from 'lucide-react';
import { useAuth } from '@/auth/AuthContext';
import { canWriteDomain } from '@/services/auth';
import {
  INITIAL_MOCK_CALLS,
  useRwiWebSocket,
  type CallState,
} from './use-rwi-websocket';
import { CallListPanel } from './call-list-panel';
import { CallDetailPanel } from './call-detail-panel';
import { TransferModal } from './transfer-modal';
import { BargeInModal } from './barge-in-modal';
import { SpeakModal } from './speak-modal';

export function RwiConsolePage() {
  const { session } = useAuth();
  const isOperatorOrAdmin = session ? canWriteDomain(session.role, 'operations') : true;

  const {
    calls, setCalls, wsConnected, wsMode, pingMs, toggleWs,
    handleBargeIn, handleSpeakSubmit, handleToggleListen,
    handleTransferSubmit, handleHangup, handleCreateSimulatedCall,
  } = useRwiWebSocket(INITIAL_MOCK_CALLS);

  // UI State
  const [selectedCallId, setSelectedCallId] = useState<string>('call-rwi-88401');
  const [filterState, setFilterState] = useState<string>('all');
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [speakModalOpen, setSpeakModalOpen] = useState(false);
  const [speakText, setSpeakText] = useState('');
  const [transferModalOpen, setTransferModalOpen] = useState(false);
  const [transferTarget, setTransferTarget] = useState('8002');
  const [bargeInConfirmOpen, setBargeInConfirmOpen] = useState(false);

  const transcriptScrollRef = useRef<HTMLDivElement>(null);
  const currentCall = calls.find((c) => c.callId === selectedCallId) || calls[0];

  useEffect(() => {
    if (transcriptScrollRef.current) {
      transcriptScrollRef.current.scrollTop = transcriptScrollRef.current.scrollHeight;
    }
  }, [currentCall?.transcripts.length]);

  const filteredCalls = calls.filter((c) => {
    if (filterState !== 'all' && c.state !== filterState) return false;
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      return (
        c.callId.toLowerCase().includes(q) ||
        c.caller.includes(q) ||
        c.callee.includes(q) ||
        c.aiAgentName.toLowerCase().includes(q)
      );
    }
    return true;
  });

  const renderStateChip = (state: CallState) => {
    switch (state) {
      case 'ringing':
        return (
          <Chip size="sm" color="warning" variant="flat" className="animate-pulse font-medium bg-warning/20 text-warning border border-warning/30">
            🔔 响铃中 (Ringing)
          </Chip>
        );
      case 'answered':
        return (
          <Chip size="sm" color="success" variant="flat" className="font-medium bg-success/20 text-success border border-success/30">
            📞 已接通 (Answered)
          </Chip>
        );
      case 'ai_active':
        return (
          <Chip size="sm" color="secondary" variant="flat" className="font-medium bg-primary/20 text-primary border border-primary/30 shadow-lg shadow-primary/20">
            ✨ AI代理交互中
          </Chip>
        );
      case 'ended':
        return (
          <Chip size="sm" color="default" variant="flat" className="font-medium bg-default-100/50 text-default-400">
            ⏹️ 已结束 (Ended)
          </Chip>
        );
    }
  };

  const formatDuration = (sec: number) => {
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
  };

  const onBargeInConfirm = () => {
    handleBargeIn(currentCall?.callId || '');
    setBargeInConfirmOpen(false);
  };
  const onSpeakSubmit = () => {
    if (!currentCall) return;
    handleSpeakSubmit(currentCall.callId, speakText);
    setSpeakText('');
    setSpeakModalOpen(false);
  };
  const onTransferSubmit = () => {
    if (!currentCall) return;
    handleTransferSubmit(currentCall.callId, transferTarget);
    setTransferModalOpen(false);
  };
  const onCreateCall = () => setSelectedCallId(handleCreateSimulatedCall());

  // KPI stats
  const activeCount = calls.filter((c) => c.state !== 'ended').length;
  const aiActiveCount = calls.filter((c) => c.state === 'ai_active').length;
  const kpiStats = [
    { Icon: PhoneCall, iconCls: 'bg-success/10 text-success', label: '并发通话数', value: activeCount, suffix: '/ 1700 Max', valCls: 'text-foreground' },
    { Icon: Bot, iconCls: 'bg-primary/10 text-primary', label: 'AI 语音 Agent 激活', value: aiActiveCount, suffix: '', valCls: 'text-foreground' },
    { Icon: Zap, iconCls: 'bg-primary/10 text-primary', label: '首包延迟 (TTFT)', value: '175 ms', suffix: '', valCls: 'text-primary' },
    { Icon: Activity, iconCls: 'bg-warning/10 text-warning', label: '媒体流 MOS 评分', value: '4.42 (高清)', suffix: '', valCls: 'text-warning' },
  ];

  return (
    <div className="flex flex-col gap-5 w-full h-full min-h-[calc(100vh-100px)]">
      {/* Top Banner & Control Bar */}
      <div className="relative overflow-hidden rounded-2xl bg-content1 border border-primary/30 p-5 shadow-2xl backdrop-blur-xl">
        <div className="relative flex flex-wrap items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="w-12 h-12 rounded-xl bg-primary flex items-center justify-center shadow-lg shadow-primary/30 text-foreground">
              <Radio className="w-6 h-6 animate-pulse" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-xl font-black tracking-tight text-foreground">
                  RWI 实时控制台 (Real-Time WebSocket Interface)
                </h1>
                <Chip size="sm" variant="flat" className="bg-primary/20 text-primary border border-primary/30 font-mono text-xs">
                  v2.4 - Full Duplex Audio
                </Chip>
              </div>
              <p className="text-xs text-foreground mt-1 flex items-center gap-2">
                <span>实时监控多路 SIP 媒体流与大模型 AI 语音 Agent 对话状态</span>
                <span className="text-default-400">•</span>
                <span className="text-primary font-mono">BargeIn 毫秒级抢断 / 双向全双工监听</span>
              </p>
            </div>
          </div>

          {/* Connection Status & Control Switchers */}
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={toggleWs}
              className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-content2 border border-default-200 text-xs font-mono text-foreground hover:bg-default-100 transition-colors cursor-pointer"
            >
              <span className={`w-2 h-2 rounded-full ${wsConnected ? 'bg-success animate-ping' : 'bg-danger'}`} />
              <span>{wsConnected ? `WS已连接 (${wsMode})` : 'WS已断开'}</span>
              <span className="text-default-400">|</span>
              <span className="text-success">{pingMs}ms</span>
            </button>

            <Button
              size="sm" variant="flat"
              className="bg-primary/20 hover:bg-primary/30 text-primary border border-primary/30 font-medium"
              startContent={<Sparkles className="w-4 h-4 text-primary" />}
              onPress={onCreateCall}
            >
              模拟呼入会话
            </Button>

            <Button
              size="sm" variant="flat"
              className="bg-default-100/10 hover:bg-default-100/20 text-foreground border border-default-200"
              startContent={<RefreshCw className="w-3.5 h-3.5" />}
              onPress={() => setCalls(INITIAL_MOCK_CALLS)}
            >
              重置状态
            </Button>
          </div>
        </div>

        {/* Realtime KPI Stat Grid */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mt-5 pt-4 border-t border-default-200">
          {kpiStats.map(({ Icon, ...kpi }) => (
            <div key={kpi.label} className="flex items-center gap-3 p-3 rounded-xl bg-default-100/10 border border-default-100/10">
              <div className={`p-2 rounded-lg ${kpi.iconCls}`}>
                <Icon className="w-5 h-5" />
              </div>
              <div>
                <div className="text-xs text-default-500 font-medium">{kpi.label}</div>
                <div className={`text-lg font-bold font-mono ${kpi.valCls}`}>
                  {kpi.value}
                  {kpi.suffix && <span className="text-xs text-default-500 font-normal"> {kpi.suffix}</span>}
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Main Workspace Layout */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-5 flex-1 min-h-0">
        <CallListPanel
          filteredCalls={filteredCalls}
          currentCallId={currentCall?.callId}
          searchQuery={searchQuery}
          filterState={filterState}
          onSelectCall={setSelectedCallId}
          onSearchChange={setSearchQuery}
          onFilterChange={setFilterState}
          renderStateChip={renderStateChip}
          formatDuration={formatDuration}
        />

        <div className="lg:col-span-8 flex flex-col gap-4 min-h-0">
          {currentCall ? (
            <CallDetailPanel
              currentCall={currentCall}
              isOperatorOrAdmin={isOperatorOrAdmin}
              transcriptScrollRef={transcriptScrollRef}
              renderStateChip={renderStateChip}
              onBargeIn={() => setBargeInConfirmOpen(true)}
              onSpeak={() => setSpeakModalOpen(true)}
              onToggleListen={() => handleToggleListen(currentCall.callId)}
              onTransfer={() => setTransferModalOpen(true)}
              onHangup={() => handleHangup(currentCall.callId)}
            />
          ) : (
            <Card shadow="sm" className="h-full flex items-center justify-center p-12 text-center text-default-400">
              <PhoneCall className="w-12 h-12 mb-3 opacity-30" />
              <p className="text-sm font-semibold">请在左侧选择需要调控的实时通话</p>
            </Card>
          )}
        </div>
      </div>

      <SpeakModal
        isOpen={speakModalOpen}
        onClose={() => setSpeakModalOpen(false)}
        speakText={speakText}
        onTextChange={setSpeakText}
        onSubmit={onSpeakSubmit}
      />
      <TransferModal
        isOpen={transferModalOpen}
        onClose={() => setTransferModalOpen(false)}
        transferTarget={transferTarget}
        onTargetChange={setTransferTarget}
        onSubmit={onTransferSubmit}
      />
      <BargeInModal
        isOpen={bargeInConfirmOpen}
        onClose={() => setBargeInConfirmOpen(false)}
        onConfirm={onBargeInConfirm}
      />
    </div>
  );
}
