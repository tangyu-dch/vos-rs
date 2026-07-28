import { useEffect, useRef, useState } from 'react';
import { Card, Chip } from '@heroui/react';
import {
  Activity,
  PhoneCall,
  Radio,
  RefreshCw,
  SignalHigh,
  SignalLow,
  SignalMedium,
  Zap,
} from 'lucide-react';
import { useAuth } from '@/auth/AuthContext';
import { canWriteDomain } from '@/services/auth';
import {
  useRwiWebSocket,
  type CallState,
  type WsConnectionState,
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
    calls,
    wsState,
    pingMs,
    reconnect,
    handleBargeIn,
    handleSpeakSubmit,
    handleToggleListen,
    handleTransferSubmit,
    handleHangup,
  } = useRwiWebSocket();

  // UI State
  const [selectedCallId, setSelectedCallId] = useState<string>('');
  const [filterState, setFilterState] = useState<string>('all');
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [speakModalOpen, setSpeakModalOpen] = useState(false);
  const [speakText, setSpeakText] = useState('');
  const [transferModalOpen, setTransferModalOpen] = useState(false);
  const [transferTarget, setTransferTarget] = useState('');
  const [bargeInConfirmOpen, setBargeInConfirmOpen] = useState(false);

  const transcriptScrollRef = useRef<HTMLDivElement>(null);
  // 当前选中的通话（若不存在则回退到第一条）
  const currentCall = calls.find((c) => c.callId === selectedCallId) || calls[0];

  // 当列表为空时清空选中
  useEffect(() => {
    if (!currentCall) setSelectedCallId('');
  }, [currentCall]);

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
        c.caller.toLowerCase().includes(q) ||
        c.callee.toLowerCase().includes(q)
      );
    }
    return true;
  });

  const renderStateChip = (state: CallState) => {
    switch (state) {
      case 'ringing':
        return (
          <Chip
            size="sm"
            color="warning"
            variant="flat"
            className="font-medium bg-warning/20 text-warning border border-warning/30 animate-pulse"
          >
            响铃中
          </Chip>
        );
      case 'answered':
        return (
          <Chip
            size="sm"
            color="success"
            variant="flat"
            className="font-medium bg-success/20 text-success border border-success/30"
          >
            已接通
          </Chip>
        );
      case 'ended':
        return (
          <Chip
            size="sm"
            color="default"
            variant="flat"
            className="font-medium bg-default-100/50 text-default-400"
          >
            已结束
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
    if (currentCall) handleBargeIn(currentCall.callId);
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
    setTransferTarget('');
    setTransferModalOpen(false);
  };

  // KPI 统计
  const activeCount = calls.filter((c) => c.state !== 'ended').length;
  const answeredCount = calls.filter((c) => c.state === 'answered').length;
  const ringingCount = calls.filter((c) => c.state === 'ringing').length;

  // WebSocket 连接状态信号图标
  const renderSignalIcon = (state: WsConnectionState) => {
    if (state === 'connected') return <SignalHigh className="w-4 h-4 text-success" />;
    if (state === 'connecting') return <SignalMedium className="w-4 h-4 text-warning animate-pulse" />;
    return <SignalLow className="w-4 h-4 text-danger" />;
  };

  const wsStateText: Record<WsConnectionState, string> = {
    connected: '已连接',
    connecting: '连接中',
    disconnected: '已断开',
  };

  const kpiStats = [
    {
      Icon: PhoneCall,
      iconCls: 'bg-success/10 text-success',
      label: '并发通话',
      value: activeCount,
      suffix: '路',
      valCls: 'text-foreground',
    },
    {
      Icon: Radio,
      iconCls: 'bg-warning/10 text-warning',
      label: '响铃中',
      value: ringingCount,
      suffix: '路',
      valCls: 'text-warning',
    },
    {
      Icon: Activity,
      iconCls: 'bg-primary/10 text-primary',
      label: '已接通',
      value: answeredCount,
      suffix: '路',
      valCls: 'text-primary',
    },
    {
      Icon: Zap,
      iconCls: 'bg-primary/10 text-primary',
      label: '链路延迟',
      value: pingMs > 0 ? `${pingMs} 毫秒` : '--',
      suffix: '',
      valCls: 'text-primary',
    },
  ];

  return (
    <div className="flex flex-col gap-5 w-full h-full min-h-[calc(100vh-100px)]">
      {/* 顶部状态栏 */}
      <div className="relative overflow-hidden rounded-2xl bg-content1 border border-primary/30 p-5 shadow-2xl backdrop-blur-xl">
        <div className="relative flex flex-wrap items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="w-12 h-12 rounded-xl bg-primary flex items-center justify-center shadow-lg shadow-primary/30 text-foreground">
              <Radio className="w-6 h-6 animate-pulse" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-xl font-black tracking-tight text-foreground">
                  实时控制台
                </h1>
                <Chip size="sm" variant="flat" className="bg-primary/20 text-primary border border-primary/30 font-mono text-xs">
                  实时
                </Chip>
              </div>
              <p className="text-xs text-foreground mt-1 flex items-center gap-2">
                <span>基于实时双工通道的呼叫事件订阅与媒体指令下发</span>
                <span className="text-default-400">•</span>
                <span className="text-primary font-mono">强插 / 播报 / 监听 / 转接 / 挂断</span>
              </p>
            </div>
          </div>

          {/* 连接状态与重连 */}
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={reconnect}
              className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-content2 border border-default-200 text-xs font-mono text-foreground hover:bg-default-100 transition-colors cursor-pointer"
              aria-label="重连实时通道"
            >
              {renderSignalIcon(wsState)}
              <span>通道 {wsStateText[wsState]}</span>
              <span className="text-default-400">|</span>
              <span className={pingMs > 0 ? 'text-success' : 'text-default-400'}>
                {pingMs > 0 ? `${pingMs}毫秒` : '--'}
              </span>
            </button>

            <button
              type="button"
              onClick={reconnect}
              className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-default-100/10 hover:bg-default-100/20 text-foreground border border-default-200 text-xs"
            >
              <RefreshCw className="w-3.5 h-3.5" />
              <span>重连</span>
            </button>
          </div>
        </div>

        {/* KPI 统计 */}
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

      {/* 主工作区：左栏通话列表 + 右栏通话详情 */}
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
              <p className="text-xs mt-1 text-default-400">建立通话后将自动出现在列表中</p>
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
