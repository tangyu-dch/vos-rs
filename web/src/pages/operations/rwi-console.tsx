import { useEffect, useRef, useState } from 'react';
import {
  Button, Card, CardBody, Chip, Input, Modal, ModalBody, ModalContent,
  ModalFooter, ModalHeader, Select, SelectItem, Tooltip,
} from '@heroui/react';
import {
  Radio, PhoneCall, PhoneOff, PhoneForwarded, Mic, Volume2,
  VolumeX, Sparkles, RefreshCw, Send,
  Bot, User, Zap, Activity, Clock, AlertTriangle,
  Search, Waves,
} from 'lucide-react';
import { motion } from 'framer-motion';
import { message } from '@/utils/toast';
import { useAuth } from '@/auth/AuthContext';
import { canWriteDomain } from '@/services/auth';

// ----------------------------------------------------------------------
// Types & Data Models
// ----------------------------------------------------------------------
export type CallState = 'ringing' | 'answered' | 'ai_active' | 'ended';
export type AiAgentStatus = 'idle' | 'listening' | 'thinking' | 'speaking' | 'barge_in';

export interface MediaStreamStats {
  codec: string;
  bitrateKbps: number;
  packetLossPercent: number;
  jitterMs: number;
  rttMs: number;
  audioLevelIn: number; // 0..100
  audioLevelOut: number; // 0..100
}

export interface AsrTranscriptItem {
  id: string;
  speaker: 'user' | 'ai' | 'system';
  text: string;
  timestamp: string;
  latencyMs?: number;
  interrupted?: boolean;
}

export interface LiveCallItem {
  callId: string;
  caller: string;
  callee: string;
  state: CallState;
  startTime: number;
  durationSec: number;
  gateway: string;
  aiAgentName: string;
  aiStatus: AiAgentStatus;
  media: MediaStreamStats;
  transcripts: AsrTranscriptItem[];
  listening: boolean;
}

// ----------------------------------------------------------------------
// Initial Mock / Demo Data
// ----------------------------------------------------------------------
const INITIAL_MOCK_CALLS: LiveCallItem[] = [
  {
    callId: 'call-rwi-88401',
    caller: '13812345678',
    callee: '400-800-9999',
    state: 'ai_active',
    startTime: Date.now() - 45000,
    durationSec: 45,
    gateway: 'GW-ALIYUN-SH-01',
    aiAgentName: '智能客服专员-小悦',
    aiStatus: 'speaking',
    listening: false,
    media: {
      codec: 'Opus/48k',
      bitrateKbps: 64,
      packetLossPercent: 0.1,
      jitterMs: 4,
      rttMs: 18,
      audioLevelIn: 35,
      audioLevelOut: 82,
    },
    transcripts: [
      { id: 't1', speaker: 'user', text: '你好，我想查询一下我上个月的账单和扣款明细。', timestamp: '00:05', latencyMs: 120 },
      { id: 't2', speaker: 'ai', text: '好的，请提供您的手机号或账户ID，我立即为您查询。', timestamp: '00:07', latencyMs: 190 },
      { id: 't3', speaker: 'user', text: '手机号就是我这个号码 13812345678。', timestamp: '00:15', latencyMs: 140 },
      { id: 't4', speaker: 'ai', text: '收到。系统查询显示，您上月总消费为 128.50 元，包含语音通话 85 分钟。', timestamp: '00:18', latencyMs: 210 },
      { id: 't5', speaker: 'user', text: '好的，请把账单发我邮箱。', timestamp: '00:32', latencyMs: 110 },
      { id: 't6', speaker: 'ai', text: '没问题，已成功触发账单推送到您的注册邮箱，请注意查收。', timestamp: '00:35', latencyMs: 175 },
    ],
  },
  {
    callId: 'call-rwi-88402',
    caller: '021-61008888',
    callee: '1001',
    state: 'ringing',
    startTime: Date.now() - 8000,
    durationSec: 8,
    gateway: 'GW-TENT-HK-02',
    aiAgentName: '自动呼叫外呼Agent',
    aiStatus: 'idle',
    listening: false,
    media: {
      codec: 'G.722/16k',
      bitrateKbps: 64,
      packetLossPercent: 0.0,
      jitterMs: 2,
      rttMs: 12,
      audioLevelIn: 0,
      audioLevelOut: 0,
    },
    transcripts: [],
  },
  {
    callId: 'call-rwi-88403',
    caller: '15900001111',
    callee: '400-800-9999',
    state: 'answered',
    startTime: Date.now() - 120000,
    durationSec: 120,
    gateway: 'GW-CHINAUNICOM-01',
    aiAgentName: 'VIP技术支持Agent',
    aiStatus: 'listening',
    listening: false,
    media: {
      codec: 'PCMU/8k',
      bitrateKbps: 64,
      packetLossPercent: 0.4,
      jitterMs: 8,
      rttMs: 25,
      audioLevelIn: 65,
      audioLevelOut: 10,
    },
    transcripts: [
      { id: 't10', speaker: 'user', text: '请问软交换节点的并发上限目前支持动态扩展吗？', timestamp: '01:20', latencyMs: 130 },
      { id: 't11', speaker: 'ai', text: '是的，Vos-rs 支持在 Kubernetes 环境中通过 HPA 自动缩放 SIP-Edge 与 Media-Edge 节点。', timestamp: '01:24', latencyMs: 180 },
    ],
  },
];

// ----------------------------------------------------------------------
// Audio Waveform / Spectrum Visualizer Component
// ----------------------------------------------------------------------
function AudioWaveform({ active, level = 50, color = 'cyan' }: { active: boolean; level?: number; color?: 'cyan' | 'violet' | 'emerald' | 'amber' }) {
  const colorMap = {
    cyan: 'bg-cyan-400 shadow-cyan-500/50',
    violet: 'bg-violet-400 shadow-violet-500/50',
    emerald: 'bg-emerald-400 shadow-emerald-500/50',
    amber: 'bg-amber-400 shadow-amber-500/50',
  };

  const barCount = 14;

  return (
    <div className="flex items-center justify-center gap-1 h-8 px-2 py-1 bg-black/40 rounded-lg border border-default-100/20 backdrop-blur-sm">
      {Array.from({ length: barCount }).map((_, i) => {
        // height calculation with dynamic pseudo-random wave if active
        const factor = Math.sin((i + 1) * 0.7) * 0.5 + 0.5;
        const barHeightPercent = active ? Math.min(100, Math.max(15, (level * factor) + (i % 3) * 15)) : 10;

        return (
          <div
            key={i}
            className={`w-1 rounded-full transition-all duration-150 ${active ? colorMap[color] : 'bg-default-600/40'}`}
            style={{
              height: `${barHeightPercent}%`,
              transitionDelay: `${i * 20}ms`,
            }}
          />
        );
      })}
    </div>
  );
}

// ----------------------------------------------------------------------
// RWI Console Component
// ----------------------------------------------------------------------
export function RwiConsolePage() {
  const { session } = useAuth();
  const isOperatorOrAdmin = session ? canWriteDomain(session.role, 'operations') : true;

  // States
  const [calls, setCalls] = useState<LiveCallItem[]>(INITIAL_MOCK_CALLS);
  const [selectedCallId, setSelectedCallId] = useState<string>('call-rwi-88401');
  const [wsConnected, setWsConnected] = useState<boolean>(true);
  const [wsMode, setWsMode] = useState<'simulated' | 'live'>('simulated');
  const [pingMs, setPingMs] = useState<number>(14);
  const [filterState, setFilterState] = useState<string>('all');
  const [searchQuery, setSearchQuery] = useState<string>('');

  // Modals for AI controls
  const [speakModalOpen, setSpeakModalOpen] = useState(false);
  const [speakText, setSpeakText] = useState('');
  const [transferModalOpen, setTransferModalOpen] = useState(false);
  const [transferTarget, setTransferTarget] = useState('8002');
  const [bargeInConfirmOpen, setBargeInConfirmOpen] = useState(false);

  // Audio elements ref for silent listening simulation
  const transcriptScrollRef = useRef<HTMLDivElement>(null);

  // Active Selected Call
  const currentCall = calls.find((c) => c.callId === selectedCallId) || calls[0];

  // Auto-scroll transcript feed
  useEffect(() => {
    if (transcriptScrollRef.current) {
      transcriptScrollRef.current.scrollTop = transcriptScrollRef.current.scrollHeight;
    }
  }, [currentCall?.transcripts.length]);

  // Periodic simulated live audio level & call duration updates
  useEffect(() => {
    const timer = setInterval(() => {
      setCalls((prevCalls) =>
        prevCalls.map((call) => {
          if (call.state === 'ended') return call;

          const isSpeaking = call.aiStatus === 'speaking';
          const isListening = call.aiStatus === 'listening';

          // Simulate fluctuating levels
          const newInLevel = call.state === 'answered' || isListening ? Math.floor(Math.random() * 50) + 20 : 0;
          const newOutLevel = isSpeaking ? Math.floor(Math.random() * 60) + 35 : 0;

          return {
            ...call,
            durationSec: call.durationSec + 1,
            media: {
              ...call.media,
              audioLevelIn: newInLevel,
              audioLevelOut: newOutLevel,
              jitterMs: Math.max(1, call.media.jitterMs + (Math.floor(Math.random() * 3) - 1)),
            },
          };
        })
      );

      // Random jitter for ping
      setPingMs((p) => Math.max(8, Math.min(45, p + (Math.floor(Math.random() * 5) - 2))));
    }, 1000);

    return () => clearInterval(timer);
  }, []);

  // Filtered call list
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

  // Action Handlers
  const handleBargeIn = (callId: string) => {
    setCalls((prev) =>
      prev.map((c) => {
        if (c.callId !== callId) return c;
        const newTranscript: AsrTranscriptItem = {
          id: `t-${Date.now()}`,
          speaker: 'system',
          text: '⚡ [BargeIn 强插信号已触发] 坐席已打断 AI 播报，接管双向媒体流通道。',
          timestamp: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
          interrupted: true,
        };
        return {
          ...c,
          aiStatus: 'barge_in',
          transcripts: [...c.transcripts, newTranscript],
        };
      })
    );
    message.warning(`已对通话 ${callId} 触发强拆/打断 (Barge-In) 指令！`);
    setBargeInConfirmOpen(false);
  };

  const handleSpeakSubmit = () => {
    if (!speakText.trim() || !currentCall) return;

    const newTranscript: AsrTranscriptItem = {
      id: `t-${Date.now()}`,
      speaker: 'ai',
      text: `[坐席指令 TTS 播报]: ${speakText.trim()}`,
      timestamp: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
      latencyMs: 95,
    };

    setCalls((prev) =>
      prev.map((c) => {
        if (c.callId !== currentCall.callId) return c;
        return {
          ...c,
          aiStatus: 'speaking',
          transcripts: [...c.transcripts, newTranscript],
        };
      })
    );

    message.success(`已向 AI Voice Agent 注入 TTS 播报命令: "${speakText.trim()}"`);
    setSpeakText('');
    setSpeakModalOpen(false);
  };

  const handleToggleListen = (callId: string) => {
    setCalls((prev) =>
      prev.map((c) => {
        if (c.callId !== callId) return c;
        const nextListening = !c.listening;
        if (nextListening) {
          message.info(`已开启通话 ${callId} 的 WebSocket 实时流静默监听通道`);
        } else {
          message.info(`已关闭通话 ${callId} 的监听`);
        }
        return { ...c, listening: nextListening };
      })
    );
  };

  const handleTransferSubmit = () => {
    if (!transferTarget.trim() || !currentCall) return;

    setCalls((prev) =>
      prev.map((c) => {
        if (c.callId !== currentCall.callId) return c;
        const newTranscript: AsrTranscriptItem = {
          id: `t-${Date.now()}`,
          speaker: 'system',
          text: `↗️ [SIP REFER 呼叫转移] 会话正转接至目标分机/网关: ${transferTarget}`,
          timestamp: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
        };
        return {
          ...c,
          state: 'ended',
          aiStatus: 'idle',
          transcripts: [...c.transcripts, newTranscript],
        };
      })
    );

    message.success(`成功向软交换网关发送 SIP REFER 转接指令 -> 目标: ${transferTarget}`);
    setTransferModalOpen(false);
  };

  const handleHangup = (callId: string) => {
    setCalls((prev) =>
      prev.map((c) => {
        if (c.callId !== callId) return c;
        const newTranscript: AsrTranscriptItem = {
          id: `t-${Date.now()}`,
          speaker: 'system',
          text: '🛑 [BYE 挂断] 坐席控制台手动释放会话 (Cause: 200 OK / Normal Release)。',
          timestamp: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
        };
        return {
          ...c,
          state: 'ended',
          aiStatus: 'idle',
          transcripts: [...c.transcripts, newTranscript],
        };
      })
    );
    message.success(`通话 ${callId} 已正常挂断`);
  };

  const handleCreateSimulatedCall = () => {
    const randomNum = Math.floor(10000000 + Math.random() * 90000000);
    const newCall: LiveCallItem = {
      callId: `call-rwi-${Math.floor(Math.random() * 90000 + 10000)}`,
      caller: `139${randomNum.toString().slice(0, 8)}`,
      callee: '400-800-9999',
      state: 'ringing',
      startTime: Date.now(),
      durationSec: 0,
      gateway: 'GW-CORE-SH-01',
      aiAgentName: '智能呼入大模型Agent',
      aiStatus: 'idle',
      listening: false,
      media: {
        codec: 'Opus/48k',
        bitrateKbps: 64,
        packetLossPercent: 0.0,
        jitterMs: 3,
        rttMs: 15,
        audioLevelIn: 0,
        audioLevelOut: 0,
      },
      transcripts: [
        {
          id: `t-init-${Date.now()}`,
          speaker: 'system',
          text: '🔔 收到 SIP INVITE 信令，呼叫正在响铃寻路中...',
          timestamp: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
        },
      ],
    };

    setCalls((prev) => [newCall, ...prev]);
    setSelectedCallId(newCall.callId);
    message.success(`已模拟生成新呼入会话 ${newCall.callId}`);
  };

  // State badge styling helper
  const renderStateChip = (state: CallState) => {
    switch (state) {
      case 'ringing':
        return (
          <Chip
            size="sm"
            color="warning"
            variant="flat"
            className="animate-pulse font-medium bg-amber-500/20 text-amber-300 border border-amber-500/30"
          >
            🔔 响铃中 (Ringing)
          </Chip>
        );
      case 'answered':
        return (
          <Chip
            size="sm"
            color="success"
            variant="flat"
            className="font-medium bg-emerald-500/20 text-emerald-300 border border-emerald-500/30"
          >
            📞 已接通 (Answered)
          </Chip>
        );
      case 'ai_active':
        return (
          <Chip
            size="sm"
            color="secondary"
            variant="flat"
            className="font-medium bg-gradient-to-r from-violet-600/30 to-cyan-500/30 text-violet-200 border border-violet-500/40 shadow-lg shadow-violet-500/20"
          >
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

  return (
    <div className="flex flex-col gap-5 w-full h-full min-h-[calc(100vh-100px)]">
      {/* ---------------------------------------------------------------------- */}
      {/* Top Banner & Control Bar */}
      {/* ---------------------------------------------------------------------- */}
      <div className="relative overflow-hidden rounded-2xl bg-gradient-to-r from-slate-900 via-indigo-950 to-slate-900 border border-indigo-500/30 p-5 shadow-2xl backdrop-blur-xl">
        <div className="absolute top-0 right-0 -mt-10 -mr-10 w-80 h-80 bg-violet-600/10 rounded-full blur-3xl pointer-events-none" />
        <div className="absolute bottom-0 left-1/3 -mb-10 w-60 h-60 bg-cyan-500/10 rounded-full blur-3xl pointer-events-none" />

        <div className="relative flex flex-wrap items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="w-12 h-12 rounded-xl bg-gradient-to-br from-indigo-500 to-violet-600 flex items-center justify-center shadow-lg shadow-indigo-500/30 text-white">
              <Radio className="w-6 h-6 animate-pulse" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-xl font-black tracking-tight text-white">
                  RWI 实时控制台 (Real-Time WebSocket Interface)
                </h1>
                <Chip
                  size="sm"
                  variant="flat"
                  className="bg-indigo-500/20 text-indigo-300 border border-indigo-500/30 font-mono text-xs"
                >
                  v2.4 - Full Duplex Audio
                </Chip>
              </div>
              <p className="text-xs text-slate-300 mt-1 flex items-center gap-2">
                <span>实时监控多路 SIP 媒体流与大模型 AI 语音 Agent 对话状态</span>
                <span className="text-slate-600">•</span>
                <span className="text-cyan-400 font-mono">BargeIn 毫秒级抢断 / 双向全双工监听</span>
              </p>
            </div>
          </div>

          {/* Connection Status & Control Switchers */}
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => {
                const nextState = !wsConnected;
                setWsConnected(nextState);
                setWsMode(nextState ? 'live' : 'simulated');
                message.info(nextState ? 'WebSocket 重连成功 (Live Mode)' : '已断开 WebSocket (Simulated Mode)');
              }}
              className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-black/40 border border-white/10 text-xs font-mono text-slate-300 hover:bg-black/60 transition-colors cursor-pointer"
            >
              <span className={`w-2 h-2 rounded-full ${wsConnected ? 'bg-emerald-400 animate-ping' : 'bg-red-500'}`} />
              <span>{wsConnected ? `WS已连接 (${wsMode})` : 'WS已断开'}</span>
              <span className="text-slate-500">|</span>
              <span className="text-emerald-400">{pingMs}ms</span>
            </button>

            <Button
              size="sm"
              variant="flat"
              className="bg-violet-600/30 hover:bg-violet-600/50 text-violet-200 border border-violet-500/40 font-medium"
              startContent={<Sparkles className="w-4 h-4 text-violet-300" />}
              onPress={handleCreateSimulatedCall}
            >
              模拟呼入会话
            </Button>

            <Button
              size="sm"
              variant="flat"
              className="bg-white/10 hover:bg-white/20 text-white border border-white/15"
              startContent={<RefreshCw className="w-3.5 h-3.5" />}
              onPress={() => setCalls(INITIAL_MOCK_CALLS)}
            >
              重置状态
            </Button>
          </div>
        </div>

        {/* Realtime KPI Stat Grid */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mt-5 pt-4 border-t border-white/10">
          <div className="flex items-center gap-3 p-3 rounded-xl bg-white/5 border border-white/5">
            <div className="p-2 rounded-lg bg-emerald-500/20 text-emerald-400">
              <PhoneCall className="w-5 h-5" />
            </div>
            <div>
              <div className="text-xs text-slate-400 font-medium">并发通话数</div>
              <div className="text-lg font-bold text-white font-mono">
                {calls.filter((c) => c.state !== 'ended').length}{' '}
                <span className="text-xs text-slate-400 font-normal">/ 1700 Max</span>
              </div>
            </div>
          </div>

          <div className="flex items-center gap-3 p-3 rounded-xl bg-white/5 border border-white/5">
            <div className="p-2 rounded-lg bg-violet-500/20 text-violet-400">
              <Bot className="w-5 h-5" />
            </div>
            <div>
              <div className="text-xs text-slate-400 font-medium">AI 语音 Agent 激活</div>
              <div className="text-lg font-bold text-white font-mono">
                {calls.filter((c) => c.state === 'ai_active').length}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-3 p-3 rounded-xl bg-white/5 border border-white/5">
            <div className="p-2 rounded-lg bg-cyan-500/20 text-cyan-400">
              <Zap className="w-5 h-5" />
            </div>
            <div>
              <div className="text-xs text-slate-400 font-medium">首包延迟 (TTFT)</div>
              <div className="text-lg font-bold text-cyan-300 font-mono">175 ms</div>
            </div>
          </div>

          <div className="flex items-center gap-3 p-3 rounded-xl bg-white/5 border border-white/5">
            <div className="p-2 rounded-lg bg-amber-500/20 text-amber-400">
              <Activity className="w-5 h-5" />
            </div>
            <div>
              <div className="text-xs text-slate-400 font-medium">媒体流 MOS 评分</div>
              <div className="text-lg font-bold text-amber-300 font-mono">4.42 (高清)</div>
            </div>
          </div>
        </div>
      </div>

      {/* ---------------------------------------------------------------------- */}
      {/* Main Workspace Layout (Left: Live Call List, Right: AI Agent Control) */}
      {/* ---------------------------------------------------------------------- */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-5 flex-1 min-h-0">
        {/* =================================================================== */}
        {/* Left Column (4 cols): Call List & Media Stream Selector */}
        {/* =================================================================== */}
        <div className="lg:col-span-4 flex flex-col gap-3 min-h-0">
          <Card shadow="sm" className="bg-content1/80 border border-default-200/60 backdrop-blur-md flex-1 flex flex-col min-h-0">
            <CardBody className="p-4 flex flex-col gap-3 min-h-0">
              {/* Search & Filter Header */}
              <div className="flex flex-col gap-2">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Radio className="w-4 h-4 text-primary" />
                    <h3 className="text-sm font-bold text-foreground">实时会话列表</h3>
                  </div>
                  <Chip size="sm" variant="flat" color="primary">
                    {filteredCalls.length} 个通话
                  </Chip>
                </div>

                <div className="flex gap-2">
                  <Input
                    size="sm"
                    placeholder="搜索 CallID / 主叫 / 被叫..."
                    value={searchQuery}
                    onValueChange={setSearchQuery}
                    startContent={<Search className="w-3.5 h-3.5 text-default-400" />}
                    isClearable
                    className="flex-1"
                  />
                  <Select
                    size="sm"
                    className="w-32"
                    selectedKeys={[filterState]}
                    onChange={(e) => setFilterState(e.target.value || 'all')}
                    aria-label="状态筛选"
                  >
                    <SelectItem key="all">全部状态</SelectItem>
                    <SelectItem key="ringing">响铃中</SelectItem>
                    <SelectItem key="answered">已接通</SelectItem>
                    <SelectItem key="ai_active">AI 交互中</SelectItem>
                    <SelectItem key="ended">已结束</SelectItem>
                  </Select>
                </div>
              </div>

              {/* Call Cards List */}
              <div className="flex-1 overflow-y-auto pr-1 space-y-2.5 min-h-0">
                {filteredCalls.length === 0 ? (
                  <div className="flex flex-col items-center justify-center p-8 text-center text-default-400 gap-2">
                    <PhoneOff className="w-8 h-8 opacity-40" />
                    <p className="text-xs font-medium">暂无匹配的实时通话</p>
                  </div>
                ) : (
                  filteredCalls.map((c) => {
                    const isSelected = c.callId === currentCall?.callId;
                    return (
                      <motion.div
                        key={c.callId}
                        whileHover={{ scale: 1.01 }}
                        transition={{ duration: 0.15 }}
                        onClick={() => setSelectedCallId(c.callId)}
                        className={`cursor-pointer p-3.5 rounded-xl border transition-all duration-200 relative overflow-hidden ${
                          isSelected
                            ? 'bg-primary/10 border-primary shadow-lg shadow-primary/10'
                            : 'bg-content2/60 border-default-200/60 hover:border-default-300'
                        }`}
                      >
                        {isSelected && (
                          <div className="absolute left-0 top-0 bottom-0 w-1 bg-primary rounded-r-full" />
                        )}

                        <div className="flex items-center justify-between gap-2 mb-2">
                          <span className="font-mono text-xs font-bold text-foreground truncate">{c.callId}</span>
                          {renderStateChip(c.state)}
                        </div>

                        <div className="grid grid-cols-2 gap-2 text-xs mb-2">
                          <div>
                            <span className="text-default-400">主叫: </span>
                            <span className="font-mono font-medium text-foreground">{c.caller}</span>
                          </div>
                          <div>
                            <span className="text-default-400">被叫: </span>
                            <span className="font-mono font-medium text-foreground">{c.callee}</span>
                          </div>
                        </div>

                        {/* Agent & Duration row */}
                        <div className="flex items-center justify-between pt-2 border-t border-default-200/40 text-tiny text-default-500">
                          <div className="flex items-center gap-1 truncate max-w-[170px]">
                            <Bot className="w-3.5 h-3.5 text-violet-400 shrink-0" />
                            <span className="truncate">{c.aiAgentName}</span>
                          </div>

                          <div className="flex items-center gap-1 font-mono text-default-400">
                            <Clock className="w-3 h-3" />
                            <span>{formatDuration(c.durationSec)}</span>
                          </div>
                        </div>

                        {/* Active waveform thumbnail if in call */}
                        {c.state !== 'ended' && (
                          <div className="mt-2 pt-2 border-t border-default-100/30 flex items-center justify-between">
                            <div className="text-[10px] text-default-400 font-mono">
                              {c.media.codec} • {c.media.packetLossPercent}% loss
                            </div>
                            <AudioWaveform
                              active={c.media.audioLevelIn > 5 || c.media.audioLevelOut > 5}
                              level={Math.max(c.media.audioLevelIn, c.media.audioLevelOut)}
                              color={c.aiStatus === 'speaking' ? 'violet' : 'cyan'}
                            />
                          </div>
                        )}
                      </motion.div>
                    );
                  })
                )}
              </div>
            </CardBody>
          </Card>
        </div>

        {/* =================================================================== */}
        {/* Right Column (8 cols): AI Voice Agent Live Control Panel */}
        {/* =================================================================== */}
        <div className="lg:col-span-8 flex flex-col gap-4 min-h-0">
          {currentCall ? (
            <Card shadow="sm" className="bg-content1/80 border border-default-200/60 backdrop-blur-md flex-1 flex flex-col min-h-0 overflow-hidden">
              {/* Header Panel for Selected Call */}
              <div className="p-4 bg-content2/80 border-b border-default-200/60 flex flex-wrap items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                  <div className="w-10 h-10 rounded-xl bg-violet-600/20 text-violet-400 border border-violet-500/30 flex items-center justify-center">
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
                  <div className="px-3 py-1.5 rounded-lg bg-violet-500/10 border border-violet-500/20 flex items-center gap-2">
                    <Sparkles className="w-4 h-4 text-violet-400 animate-spin" />
                    <span className="text-xs font-semibold text-violet-300">
                      {currentCall.aiStatus === 'speaking' && '🗣️ AI正在播报发言'}
                      {currentCall.aiStatus === 'listening' && '👂 AI正在倾听用户'}
                      {currentCall.aiStatus === 'thinking' && '🧠 AI大模型推理中...'}
                      {currentCall.aiStatus === 'barge_in' && '⚡ 已被坐席抢断接管'}
                      {currentCall.aiStatus === 'idle' && '💤 待命'}
                    </span>
                  </div>
                </div>
              </div>

              <CardBody className="p-4 flex flex-col gap-4 flex-1 min-h-0 overflow-y-auto">
                {/* ------------------------------------------------------------------ */}
                {/* Interactive AI Agent Control Action Bar */}
                {/* ------------------------------------------------------------------ */}
                <div className="p-4 rounded-xl bg-gradient-to-r from-slate-900/90 to-indigo-950/90 border border-indigo-500/30 shadow-xl">
                  <div className="text-xs font-semibold text-indigo-300 uppercase tracking-wider mb-3 flex items-center justify-between">
                    <span className="flex items-center gap-1.5">
                      <Zap className="w-4 h-4 text-amber-400" />
                      AI Voice Agent 实时强控指令面板
                    </span>
                    <span className="text-[10px] text-indigo-400 font-normal">全双工 RTP 双向注入</span>
                  </div>

                  <div className="grid grid-cols-2 sm:grid-cols-5 gap-2.5">
                    {/* 1. BargeIn / Interrupt */}
                    <Tooltip content="立即切断 AI 当前播报，向媒体链路注入打断标记并接管" placement="top">
                      <Button
                        color="danger"
                        variant="shadow"
                        size="md"
                        disabled={!isOperatorOrAdmin || currentCall.state === 'ended'}
                        onPress={() => setBargeInConfirmOpen(true)}
                        startContent={<Zap className="w-4 h-4" />}
                        className="font-bold bg-gradient-to-r from-red-600 to-rose-600 text-white shadow-red-600/30"
                      >
                        BargeIn 强插
                      </Button>
                    </Tooltip>

                    {/* 2. Speak / TTS Injection */}
                    <Tooltip content="自定义输入文本并由 AI Voice Agent 立即合成语音播报" placement="top">
                      <Button
                        color="secondary"
                        variant="flat"
                        size="md"
                        disabled={!isOperatorOrAdmin || currentCall.state === 'ended'}
                        onPress={() => setSpeakModalOpen(true)}
                        startContent={<Mic className="w-4 h-4" />}
                        className="font-bold bg-violet-600/30 text-violet-200 border border-violet-500/40 hover:bg-violet-600/50"
                      >
                        Speak 合成
                      </Button>
                    </Tooltip>

                    {/* 3. Listen / Silent Tap */}
                    <Tooltip content="启用/关闭本地静默监听通道，实时收听双方 RTP 音频" placement="top">
                      <Button
                        color={currentCall.listening ? 'warning' : 'primary'}
                        variant={currentCall.listening ? 'solid' : 'flat'}
                        size="md"
                        disabled={currentCall.state === 'ended'}
                        onPress={() => handleToggleListen(currentCall.callId)}
                        startContent={currentCall.listening ? <VolumeX className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
                        className="font-bold"
                      >
                        {currentCall.listening ? '取消监听' : 'Listen 监听'}
                      </Button>
                    </Tooltip>

                    {/* 4. Transfer */}
                    <Tooltip content="发送 SIP REFER 盲转或协同转接至指定座席分机" placement="top">
                      <Button
                        color="success"
                        variant="flat"
                        size="md"
                        disabled={!isOperatorOrAdmin || currentCall.state === 'ended'}
                        onPress={() => setTransferModalOpen(true)}
                        startContent={<PhoneForwarded className="w-4 h-4" />}
                        className="font-bold bg-emerald-600/20 text-emerald-300 border border-emerald-500/40 hover:bg-emerald-600/40"
                      >
                        Transfer 转接
                      </Button>
                    </Tooltip>

                    {/* 5. Hangup */}
                    <Tooltip content="立即强拆并挂断当前 SIP 会话" placement="top">
                      <Button
                        color="danger"
                        variant="flat"
                        size="md"
                        disabled={!isOperatorOrAdmin || currentCall.state === 'ended'}
                        onPress={() => handleHangup(currentCall.callId)}
                        startContent={<PhoneOff className="w-4 h-4" />}
                        className="font-bold bg-red-500/20 text-red-300 border border-red-500/30 hover:bg-red-500/40 col-span-2 sm:col-span-1"
                      >
                        Hangup 挂断
                      </Button>
                    </Tooltip>
                  </div>
                </div>

                {/* ------------------------------------------------------------------ */}
                {/* Media Stream Realtime Spectrum & Metrics Panel */}
                {/* ------------------------------------------------------------------ */}
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  {/* Inbound Stream Metrics (Caller -> System) */}
                  <div className="p-3.5 rounded-xl bg-content2/70 border border-default-200/50 flex flex-col gap-2">
                    <div className="flex items-center justify-between">
                      <span className="text-xs font-bold text-foreground flex items-center gap-1.5">
                        <User className="w-3.5 h-3.5 text-cyan-400" />
                        主叫上行 RTP 媒体流 (Inbound)
                      </span>
                      <span className="font-mono text-tiny text-cyan-400">{currentCall.media.codec}</span>
                    </div>

                    <div className="flex items-center justify-between gap-3">
                      <AudioWaveform
                        active={currentCall.media.audioLevelIn > 5}
                        level={currentCall.media.audioLevelIn}
                        color="cyan"
                      />
                      <div className="text-right font-mono text-tiny text-default-400 space-y-0.5">
                        <div>Bitrate: <span className="text-foreground">{currentCall.media.bitrateKbps} kbps</span></div>
                        <div>Loss: <span className="text-emerald-400">{currentCall.media.packetLossPercent}%</span></div>
                      </div>
                    </div>
                  </div>

                  {/* Outbound Stream Metrics (AI Agent -> Caller) */}
                  <div className="p-3.5 rounded-xl bg-content2/70 border border-default-200/50 flex flex-col gap-2">
                    <div className="flex items-center justify-between">
                      <span className="text-xs font-bold text-foreground flex items-center gap-1.5">
                        <Bot className="w-3.5 h-3.5 text-violet-400" />
                        AI 下行 TTS 媒体流 (Outbound)
                      </span>
                      <span className="font-mono text-tiny text-violet-400">Opus/48k (Low Latency)</span>
                    </div>

                    <div className="flex items-center justify-between gap-3">
                      <AudioWaveform
                        active={currentCall.media.audioLevelOut > 5}
                        level={currentCall.media.audioLevelOut}
                        color="violet"
                      />
                      <div className="text-right font-mono text-tiny text-default-400 space-y-0.5">
                        <div>Jitter: <span className="text-foreground">{currentCall.media.jitterMs} ms</span></div>
                        <div>RTT: <span className="text-cyan-400">{currentCall.media.rttMs} ms</span></div>
                      </div>
                    </div>
                  </div>
                </div>

                {/* ------------------------------------------------------------------ */}
                {/* Live ASR Subtitle & Transcript Stream */}
                {/* ------------------------------------------------------------------ */}
                <div className="flex-1 flex flex-col min-h-[280px] bg-black/40 rounded-xl border border-default-200/50 p-4">
                  <div className="flex items-center justify-between pb-3 mb-3 border-b border-white/10">
                    <div className="flex items-center gap-2">
                      <Waves className="w-4 h-4 text-violet-400 animate-pulse" />
                      <h4 className="text-xs font-bold text-foreground">实时 ASR 语音识别字幕与对话流</h4>
                    </div>
                    <div className="text-[10px] text-default-400 font-mono">
                      WebSocket Event: <span className="text-emerald-400">asr_stream_active</span>
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
                                  ? 'bg-red-500/20 text-red-300 border-red-500/30'
                                  : 'bg-indigo-500/20 text-indigo-300 border-indigo-500/30'
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
                                ? 'bg-cyan-500/20 text-cyan-300 border border-cyan-500/40'
                                : 'bg-violet-600/30 text-violet-200 border border-violet-500/40'
                            }`}>
                              {isUser ? <User className="w-4 h-4" /> : <Bot className="w-4 h-4" />}
                            </div>

                            <div>
                              <div className={`flex items-center gap-2 mb-1 text-[10px] text-default-400 ${isUser ? '' : 'flex-row-reverse'}`}>
                                <span className="font-semibold">{isUser ? '用户 (Caller)' : currentCall.aiAgentName}</span>
                                <span>•</span>
                                <span className="font-mono">{t.timestamp}</span>
                                {t.latencyMs && (
                                  <span className="text-emerald-400 font-mono">({t.latencyMs}ms)</span>
                                )}
                              </div>

                              <div className={`p-3 rounded-2xl text-xs leading-relaxed ${
                                isUser
                                  ? 'bg-slate-800/80 text-slate-100 rounded-tl-none border border-slate-700/60'
                                  : 'bg-violet-950/80 text-violet-100 rounded-tr-none border border-violet-700/60 shadow-lg shadow-violet-950/40'
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
          ) : (
            <Card shadow="sm" className="h-full flex items-center justify-center p-12 text-center text-default-400">
              <PhoneCall className="w-12 h-12 mb-3 opacity-30" />
              <p className="text-sm font-semibold">请在左侧选择需要调控的实时通话</p>
            </Card>
          )}
        </div>
      </div>

      {/* ---------------------------------------------------------------------- */}
      {/* Modals for Interactive Controls */}
      {/* ---------------------------------------------------------------------- */}

      {/* Speak TTS Modal */}
      <Modal isOpen={speakModalOpen} onClose={() => setSpeakModalOpen(false)}>
        <ModalContent>
          <ModalHeader className="flex items-center gap-2">
            <Mic className="w-5 h-5 text-violet-400" />
            <span>AI Voice Agent 注入 TTS 合成播报</span>
          </ModalHeader>
          <ModalBody className="gap-3">
            <p className="text-xs text-default-500">
              手动输入的文本将通过实时 TTS 引擎合成音频并直接注入当前通话媒体通道。
            </p>
            <Input
              label="TTS 播报文本"
              placeholder="请输入需要 AI 语音播报的内容..."
              value={speakText}
              onValueChange={setSpeakText}
              autoFocus
            />
            <div className="flex flex-wrap gap-1.5 pt-2">
              <span className="text-tiny text-default-400 w-full mb-1">快捷回复预设:</span>
              {[
                '请您稍等，我立即为您转接人工客服',
                '您的身份验证已通过，请问还有其他需求吗？',
                '十分抱歉给您带来不便，我们将优先处理。',
              ].map((phrase) => (
                <Chip
                  key={phrase}
                  size="sm"
                  variant="flat"
                  className="cursor-pointer hover:bg-violet-500/20"
                  onClick={() => setSpeakText(phrase)}
                >
                  {phrase}
                </Chip>
              ))}
            </div>
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onPress={() => setSpeakModalOpen(false)}>
              取消
            </Button>
            <Button color="secondary" onPress={handleSpeakSubmit} startContent={<Send className="w-4 h-4" />}>
              确认发送播报
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>

      {/* Transfer Call Modal */}
      <Modal isOpen={transferModalOpen} onClose={() => setTransferModalOpen(false)}>
        <ModalContent>
          <ModalHeader className="flex items-center gap-2">
            <PhoneForwarded className="w-5 h-5 text-emerald-400" />
            <span>执行 SIP REFER 呼叫转移 (Transfer)</span>
          </ModalHeader>
          <ModalBody className="gap-3">
            <p className="text-xs text-default-500">
              将当前通话盲转 (Blind Transfer) 至座席分机、队列或外部 PSTN 中继号码。
            </p>
            <Input
              label="目标分机 / 号码"
              placeholder="如 8002, 1001, 或外部手机号"
              value={transferTarget}
              onValueChange={setTransferTarget}
              autoFocus
            />
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onPress={() => setTransferModalOpen(false)}>
              取消
            </Button>
            <Button color="success" onPress={handleTransferSubmit}>
              确认转接
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>

      {/* BargeIn Confirmation Modal */}
      <Modal isOpen={bargeInConfirmOpen} onClose={() => setBargeInConfirmOpen(false)}>
        <ModalContent>
          <ModalHeader className="flex items-center gap-2 text-danger">
            <AlertTriangle className="w-5 h-5" />
            <span>确认执行 BargeIn 强插抢断？</span>
          </ModalHeader>
          <ModalBody>
            <p className="text-xs text-default-600">
              该指令会立即中断 AI Voice Agent 的当前合成输出，强制将音频流切换为坐席直连模式。
            </p>
          </ModalBody>
          <ModalFooter>
            <Button variant="flat" onPress={() => setBargeInConfirmOpen(false)}>
              取消
            </Button>
            <Button color="danger" onPress={() => handleBargeIn(currentCall?.callId || '')}>
              确认强插 (BargeIn)
            </Button>
          </ModalFooter>
        </ModalContent>
      </Modal>
    </div>
  );
}
