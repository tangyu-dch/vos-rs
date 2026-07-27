import { useEffect, useState } from 'react';
import { message } from '@/utils/toast';

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
export const INITIAL_MOCK_CALLS: LiveCallItem[] = [
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
// WebSocket (Simulated) Hook
// ----------------------------------------------------------------------
export interface UseRwiWebSocketResult {
  calls: LiveCallItem[];
  setCalls: React.Dispatch<React.SetStateAction<LiveCallItem[]>>;
  wsConnected: boolean;
  wsMode: 'simulated' | 'live';
  pingMs: number;
  toggleWs: () => void;
  handleBargeIn: (callId: string) => void;
  handleSpeakSubmit: (callId: string, text: string) => void;
  handleToggleListen: (callId: string) => void;
  handleTransferSubmit: (callId: string, target: string) => void;
  handleHangup: (callId: string) => void;
  handleCreateSimulatedCall: () => string;
}

export function useRwiWebSocket(initialCalls: LiveCallItem[]): UseRwiWebSocketResult {
  const [calls, setCalls] = useState<LiveCallItem[]>(initialCalls);
  const [wsConnected, setWsConnected] = useState<boolean>(true);
  const [wsMode, setWsMode] = useState<'simulated' | 'live'>('simulated');
  const [pingMs, setPingMs] = useState<number>(14);

  // Periodic simulated live audio level & call duration updates
  useEffect(() => {
    const timer = setInterval(() => {
      setCalls((prevCalls) =>
        prevCalls.map((call) => {
          if (call.state === 'ended') return call;

          const isSpeaking = call.aiStatus === 'speaking';
          const isListening = call.aiStatus === 'listening';

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

      setPingMs((p) => Math.max(8, Math.min(45, p + (Math.floor(Math.random() * 5) - 2))));
    }, 1000);

    return () => clearInterval(timer);
  }, []);

  const toggleWs = () => {
    const nextState = !wsConnected;
    setWsConnected(nextState);
    setWsMode(nextState ? 'live' : 'simulated');
    message.info(nextState ? 'WebSocket 重连成功 (Live Mode)' : '已断开 WebSocket (Simulated Mode)');
  };

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
  };

  const handleSpeakSubmit = (callId: string, text: string) => {
    if (!text.trim()) return;

    const newTranscript: AsrTranscriptItem = {
      id: `t-${Date.now()}`,
      speaker: 'ai',
      text: `[坐席指令 TTS 播报]: ${text.trim()}`,
      timestamp: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
      latencyMs: 95,
    };

    setCalls((prev) =>
      prev.map((c) => {
        if (c.callId !== callId) return c;
        return {
          ...c,
          aiStatus: 'speaking',
          transcripts: [...c.transcripts, newTranscript],
        };
      })
    );

    message.success(`已向 AI Voice Agent 注入 TTS 播报命令: "${text.trim()}"`);
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

  const handleTransferSubmit = (callId: string, target: string) => {
    if (!target.trim()) return;

    setCalls((prev) =>
      prev.map((c) => {
        if (c.callId !== callId) return c;
        const newTranscript: AsrTranscriptItem = {
          id: `t-${Date.now()}`,
          speaker: 'system',
          text: `↗️ [SIP REFER 呼叫转移] 会话正转接至目标分机/网关: ${target}`,
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

    message.success(`成功向软交换网关发送 SIP REFER 转接指令 -> 目标: ${target}`);
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

  const handleCreateSimulatedCall = (): string => {
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
    message.success(`已模拟生成新呼入会话 ${newCall.callId}`);
    return newCall.callId;
  };

  return {
    calls,
    setCalls,
    wsConnected,
    wsMode,
    pingMs,
    toggleWs,
    handleBargeIn,
    handleSpeakSubmit,
    handleToggleListen,
    handleTransferSubmit,
    handleHangup,
    handleCreateSimulatedCall,
  };
}
