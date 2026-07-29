import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '@/services/client';
import { getAccessToken } from '@/services/auth';
import { message } from '@/utils/toast';

// ----------------------------------------------------------------------
// 常量配置
// ----------------------------------------------------------------------

/**
 * RWI WebSocket 端点。
 * 默认指向 api-server 开发环境地址；如需覆盖，可在编译期通过 Vite 环境变量重新构建。
 */
const RWI_WS_URL: string = 'ws://localhost:8081/rwi/v1/ws';

/** 心跳发送间隔（毫秒） */
const HEARTBEAT_INTERVAL_MS = 30_000;

/** 自动重连初始退避（毫秒） */
const RECONNECT_BASE_MS = 1_000;

/** 自动重连最大退避（毫秒） */
const RECONNECT_MAX_MS = 30_000;

/** 通话结束后从列表移除的延迟（毫秒） */
const ENDED_REMOVE_DELAY_MS = 5_000;

/** 心跳超时阈值（毫秒）：超过此时间未收到 Pong 则认为连接已断开 */
const PONG_TIMEOUT_MS = 10_000;

// ----------------------------------------------------------------------
// 类型定义
// ----------------------------------------------------------------------

export type CallState = 'ringing' | 'answered' | 'ended';

export type WsConnectionState = 'connected' | 'connecting' | 'disconnected';

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
  speaker: 'user' | 'system';
  text: string;
  timestamp: string;
  latencyMs?: number;
  interrupted?: boolean;
}

export interface LiveCallItem {
  callId: string;
  caller: string;
  callee: string;
  direction: 'inbound' | 'outbound';
  state: CallState;
  startTime: number;
  durationSec: number;
  gateway?: string;
  /** 后端 MediaEvent 仅推送部分字段，故使用 Partial */
  media: Partial<MediaStreamStats>;
  transcripts: AsrTranscriptItem[];
  /** 是否处于监听状态（前端维护，非后端推送） */
  listening: boolean;
}

/** 后端 /calls/active 列表项 */
interface ActiveCallDto {
  call_id?: string;
  caller?: string;
  callee?: string;
  state?: string;
  started_at_ms?: number;
  gateway?: string;
}

/** RWI 事件类型字面量 */
type RwiEventType =
  'call_started' | 'call_ringing' | 'call_answered' | 'call_ended' | 'media_event';

/** RWI 指令类型字面量 */
type RwiCommandType = 'barge_in' | 'speak' | 'listen' | 'transfer' | 'hangup';

/** RWI WebSocket 消息包（与 call-core::rwi::RwiMessage 对应） */
interface RwiMessage {
  id: string;
  version: string;
  event?: RwiEventType;
  command?: RwiCommandType;
  data?: Record<string, unknown>;
}

// ----------------------------------------------------------------------
// 工具函数
// ----------------------------------------------------------------------

/** 生成唯一消息 ID */
function genUuid(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `msg-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

/** 将后端 state 字符串归一化为前端 CallState */
function normalizeState(raw: string | undefined): CallState {
  const s = String(raw || '').toLowerCase();
  if (s === 'answered' || s === 'active' || s === 'in_call' || s === 'in_call_early')
    return 'answered';
  if (s === 'ended' || s === 'terminated' || s === 'completed') return 'ended';
  return 'ringing';
}

/** 将后端 direction 字符串归一化 */
function normalizeDirection(raw: string | undefined): 'inbound' | 'outbound' {
  return String(raw || 'inbound').toLowerCase() === 'outbound' ? 'outbound' : 'inbound';
}

/** 把活跃通话 DTO 转换为前端模型 */
function activeCallToLiveItem(dto: ActiveCallDto): LiveCallItem {
  const startedAt = dto.started_at_ms ?? Date.now();
  const state = normalizeState(dto.state);
  return {
    callId: String(dto.call_id ?? ''),
    caller: String(dto.caller ?? ''),
    callee: String(dto.callee ?? ''),
    direction: 'inbound',
    state,
    startTime: startedAt,
    durationSec: state === 'ended' ? 0 : Math.max(0, Math.floor((Date.now() - startedAt) / 1000)),
    gateway: dto.gateway || undefined,
    media: {},
    transcripts: [
      {
        id: `sync-${dto.call_id ?? startedAt}`,
        speaker: 'system',
        text:
          state === 'answered' ? '已同步当前通话，通话处于接通状态' : '已同步当前通话，等待接通',
        timestamp: nowTimestamp(),
      },
    ],
    listening: false,
  };
}

/** 生成时间戳字符串（HH:MM:SS） */
function nowTimestamp(): string {
  return new Date().toLocaleTimeString('zh-CN', { hour12: false });
}

/** 在通话内追加系统转写 */
function appendSystemTranscript(
  call: LiveCallItem,
  text: string,
  interrupted = false,
): LiveCallItem {
  const item: AsrTranscriptItem = {
    id: `t-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
    speaker: 'system',
    text,
    timestamp: nowTimestamp(),
    interrupted,
  };
  return { ...call, transcripts: [...call.transcripts, item] };
}

/** 将 Bearer token 以 query 参数形式追加到 WebSocket URL */
function appendTokenToUrl(url: string, token: string | null): string {
  if (!token) return url;
  const sep = url.includes('?') ? '&' : '?';
  return `${url}${sep}access_token=${encodeURIComponent(token)}`;
}

// ----------------------------------------------------------------------
// Hook 返回值
// ----------------------------------------------------------------------

export interface UseRwiWebSocketResult {
  calls: LiveCallItem[];
  wsState: WsConnectionState;
  pingMs: number;
  /** 主动断开/重连 */
  reconnect: () => void;
  handleBargeIn: (callId: string) => void;
  handleSpeakSubmit: (callId: string, text: string) => void;
  handleToggleListen: (callId: string) => void;
  handleTransferSubmit: (callId: string, target: string) => void;
  handleHangup: (callId: string) => void;
}

// ----------------------------------------------------------------------
// Hook 实现
// ----------------------------------------------------------------------

export function useRwiWebSocket(): UseRwiWebSocketResult {
  const [calls, setCalls] = useState<LiveCallItem[]>([]);
  const [wsState, setWsState] = useState<WsConnectionState>('disconnected');
  const [pingMs, setPingMs] = useState<number>(0);
  // 用于在面板中触发"监听状态"重渲染（每秒同步一次）
  const [, setListeningVersion] = useState(0);

  // 持有 WebSocket 实例与各定时器
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const heartbeatTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const reconnectAttemptRef = useRef<number>(0);
  const lastPongAtRef = useRef<number>(Date.now());

  // 已结束通话的移除定时器
  const removeTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  // 监听状态本地缓存（用 state 镜像驱动 UI 渲染）
  const listeningSetRef = useRef<Set<string>>(new Set());

  // 把 handler 引用放进 ref，避免 WebSocket 回调闭包依赖循环
  const handleRwiEventRef = useRef<(type: RwiEventType, data: Record<string, unknown>) => void>(
    () => {},
  );
  const scheduleReconnectRef = useRef<() => void>(() => {});

  // ----------------------------------------------------------------
  // 内部：处理接收到的 RWI 事件
  // ----------------------------------------------------------------
  const handleRwiEvent = useCallback((type: RwiEventType, data: Record<string, unknown>) => {
    const callId = String(data?.call_id ?? '');
    if (!callId) return;
    const ts = Number(data?.timestamp_ms ?? Date.now());

    setCalls((prev) => {
      switch (type) {
        case 'call_started': {
          // REST 初始化可能先同步到同一通话，此时补充事件而不是丢弃。
          if (prev.some((c) => c.callId === callId)) {
            return prev.map((call) =>
              call.callId === callId
                ? appendSystemTranscript(call, '收到呼叫建立事件，正在选择路由')
                : call,
            );
          }
          const item: LiveCallItem = {
            callId,
            caller: String(data?.caller ?? ''),
            callee: String(data?.callee ?? ''),
            direction: normalizeDirection(String(data?.direction ?? 'inbound')),
            state: 'ringing',
            startTime: ts,
            durationSec: 0,
            gateway: undefined,
            media: {},
            transcripts: [
              {
                id: `t-${ts}`,
                speaker: 'system',
                text: '收到呼叫请求，正在建立通话…',
                timestamp: nowTimestamp(),
              },
            ],
            listening: false,
          };
          return [item, ...prev];
        }
        case 'call_ringing': {
          return prev.map((call) =>
            call.callId === callId
              ? appendSystemTranscript({ ...call, state: 'ringing' }, '被叫振铃中')
              : call,
          );
        }
        case 'call_answered': {
          return prev.map((call) =>
            call.callId === callId
              ? appendSystemTranscript({ ...call, state: 'answered' }, '通话已接通')
              : call,
          );
        }
        case 'call_ended': {
          const duration = Number(data?.duration_secs ?? 0);
          const reason = String(data?.reason ?? 'normal');
          return prev.map((c) => {
            if (c.callId !== callId) return c;
            const updated = appendSystemTranscript(
              { ...c, state: 'ended', durationSec: duration || c.durationSec },
              `通话已结束（原因: ${reason}）`,
            );
            // 5 秒后从列表移除
            const existing = removeTimersRef.current.get(callId);
            if (existing) clearTimeout(existing);
            const timer = setTimeout(() => {
              setCalls((curr) => curr.filter((item) => item.callId !== callId));
              removeTimersRef.current.delete(callId);
              listeningSetRef.current.delete(callId);
            }, ENDED_REMOVE_DELAY_MS);
            removeTimersRef.current.set(callId, timer);
            return updated;
          });
        }
        case 'media_event': {
          const eventType = String(data?.event_type ?? '');
          const payloadRaw = String(data?.payload ?? '');
          let payload: Record<string, unknown> = {};
          if (payloadRaw) {
            try {
              payload = JSON.parse(payloadRaw) as Record<string, unknown>;
            } catch {
              payload = {};
            }
          }
          return prev.map((c) => {
            if (c.callId !== callId) return c;
            const nextMedia: Partial<MediaStreamStats> = { ...c.media };
            if (typeof payload.codec === 'string') nextMedia.codec = payload.codec;
            if (typeof payload.bitrate_kbps === 'number')
              nextMedia.bitrateKbps = payload.bitrate_kbps;
            else if (typeof payload.bitrateKbps === 'number')
              nextMedia.bitrateKbps = payload.bitrateKbps;
            if (typeof payload.packet_loss_percent === 'number')
              nextMedia.packetLossPercent = payload.packet_loss_percent;
            else if (typeof payload.packetLossPercent === 'number')
              nextMedia.packetLossPercent = payload.packetLossPercent;
            if (typeof payload.jitter_ms === 'number') nextMedia.jitterMs = payload.jitter_ms;
            else if (typeof payload.jitterMs === 'number') nextMedia.jitterMs = payload.jitterMs;
            if (typeof payload.rtt_ms === 'number') nextMedia.rttMs = payload.rtt_ms;
            else if (typeof payload.rttMs === 'number') nextMedia.rttMs = payload.rttMs;
            if (typeof payload.audio_level_in === 'number')
              nextMedia.audioLevelIn = payload.audio_level_in;
            else if (typeof payload.audioLevelIn === 'number')
              nextMedia.audioLevelIn = payload.audioLevelIn;
            if (typeof payload.audio_level_out === 'number')
              nextMedia.audioLevelOut = payload.audio_level_out;
            else if (typeof payload.audioLevelOut === 'number')
              nextMedia.audioLevelOut = payload.audioLevelOut;

            // DTMF 事件追加转写
            if (eventType === 'dtmf_received') {
              const digit = String(payload.digit ?? payload.dtmf ?? '');
              if (digit) {
                return {
                  ...c,
                  media: nextMedia,
                  transcripts: [
                    ...c.transcripts,
                    {
                      id: `t-${ts}-${Math.random().toString(36).slice(2, 6)}`,
                      speaker: 'system',
                      text: `收到按键: ${digit}`,
                      timestamp: nowTimestamp(),
                    },
                  ],
                };
              }
            }
            return { ...c, media: nextMedia };
          });
        }
        default:
          return prev;
      }
    });
  }, []);

  // 同步最新 handler 到 ref
  useEffect(() => {
    handleRwiEventRef.current = handleRwiEvent;
  }, [handleRwiEvent]);

  // ----------------------------------------------------------------
  // 内部：清理心跳与重连定时器
  // ----------------------------------------------------------------
  const clearTimers = useCallback(() => {
    if (heartbeatTimerRef.current) {
      clearInterval(heartbeatTimerRef.current);
      heartbeatTimerRef.current = null;
    }
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  // ----------------------------------------------------------------
  // 内部：自动重连（指数退避，最大 30s）
  // ----------------------------------------------------------------
  const scheduleReconnect = useCallback(() => {
    if (reconnectTimerRef.current) return;
    const attempt = reconnectAttemptRef.current + 1;
    reconnectAttemptRef.current = attempt;
    const delay = Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * Math.pow(2, attempt - 1));
    reconnectTimerRef.current = setTimeout(() => {
      reconnectTimerRef.current = null;
      // 通过 ref 调用 connect，避免依赖循环
      connectRef.current();
    }, delay);
  }, []);

  // 同步 scheduleReconnect 到 ref（供 connect 闭包使用）
  useEffect(() => {
    scheduleReconnectRef.current = scheduleReconnect;
  }, [scheduleReconnect]);

  // ----------------------------------------------------------------
  // 内部：建立 WebSocket 连接
  // ----------------------------------------------------------------
  const connect = useCallback(() => {
    // 关闭旧连接
    if (wsRef.current) {
      try {
        wsRef.current.onclose = null;
        wsRef.current.onerror = null;
        wsRef.current.onmessage = null;
        wsRef.current.onopen = null;
        wsRef.current.close();
      } catch {
        /* noop */
      }
      wsRef.current = null;
    }
    clearTimers();

    setWsState('connecting');

    let ws: WebSocket;
    try {
      // 携带 Bearer token（与 HTTP 一致）
      const urlWithAuth = appendTokenToUrl(RWI_WS_URL, getAccessToken());
      ws = new WebSocket(urlWithAuth);
    } catch (e) {
      message.error('WebSocket 创建失败：' + (e instanceof Error ? e.message : String(e)));
      setWsState('disconnected');
      scheduleReconnectRef.current();
      return;
    }
    wsRef.current = ws;

    ws.onopen = () => {
      reconnectAttemptRef.current = 0;
      setWsState('connected');
      lastPongAtRef.current = Date.now();
      // 启动心跳
      heartbeatTimerRef.current = setInterval(() => {
        const sock = wsRef.current;
        if (!sock || sock.readyState !== WebSocket.OPEN) return;
        const t0 = Date.now();
        try {
          sock.send(JSON.stringify({ type: 'ping', ts: t0 }));
        } catch {
          /* noop */
        }
        // 心跳超时检测
        if (Date.now() - lastPongAtRef.current > PONG_TIMEOUT_MS) {
          try {
            sock.close();
          } catch {
            /* noop */
          }
        }
      }, HEARTBEAT_INTERVAL_MS);
    };

    ws.onmessage = (ev) => {
      const raw = typeof ev.data === 'string' ? ev.data : '';
      if (!raw) return;
      // 处理 Pong
      if (raw === 'pong' || raw.startsWith('{"type":"pong"')) {
        lastPongAtRef.current = Date.now();
        try {
          const parsed = JSON.parse(raw) as { ts?: number };
          if (typeof parsed.ts === 'number') {
            setPingMs(Math.max(0, Date.now() - parsed.ts));
          } else {
            setPingMs((prev) => (prev === 0 ? 1 : prev));
          }
        } catch {
          /* noop */
        }
        return;
      }
      // 解析 RWI 消息
      let msg: RwiMessage;
      try {
        msg = JSON.parse(raw) as RwiMessage;
      } catch {
        return;
      }
      if (msg.event) {
        handleRwiEventRef.current(msg.event, (msg.data ?? {}) as Record<string, unknown>);
      }
      // 后端 ACK（msg.command）忽略
    };

    ws.onerror = () => {
      // 错误本身不弹窗，由 onclose 触发重连
    };

    ws.onclose = () => {
      setWsState('disconnected');
      setPingMs(0);
      clearTimers();
      scheduleReconnectRef.current();
    };
  }, [clearTimers]);

  // connect 也通过 ref 暴露给 scheduleReconnect
  const connectRef = useRef(connect);
  useEffect(() => {
    connectRef.current = connect;
  }, [connect]);

  // ----------------------------------------------------------------
  // 初始数据加载 + 建立 WebSocket
  // ----------------------------------------------------------------
  useEffect(() => {
    let cancelled = false;
    async function bootstrap() {
      try {
        const data = await api.get<ActiveCallDto[]>('/calls/active');
        if (cancelled) return;
        const items = (Array.isArray(data) ? data : []).map(activeCallToLiveItem);
        setCalls(items);
      } catch (e) {
        // 初始加载失败不阻断 WebSocket 建立
        message.warning('初始活跃通话加载失败：' + (e instanceof Error ? e.message : String(e)));
      } finally {
        if (!cancelled) connect();
      }
    }
    void bootstrap();

    return () => {
      cancelled = true;
      // 清理 WebSocket 与所有定时器
      if (wsRef.current) {
        try {
          wsRef.current.onclose = null;
          wsRef.current.close();
        } catch {
          /* noop */
        }
        wsRef.current = null;
      }
      clearTimers();
      removeTimersRef.current.forEach((t) => clearTimeout(t));
      removeTimersRef.current.clear();
    };
  }, [connect, clearTimers]);

  // ----------------------------------------------------------------
  // 每秒更新通话时长 + 同步监听状态
  // ----------------------------------------------------------------
  useEffect(() => {
    const timer = setInterval(() => {
      setCalls((prev) =>
        prev.map((c) => {
          if (c.state === 'ended') return c;
          const newDuration = Math.max(0, Math.floor((Date.now() - c.startTime) / 1000));
          // 同步 listening 状态（来自 ref 集合）
          const newListening = listeningSetRef.current.has(c.callId);
          if (newDuration === c.durationSec && newListening === c.listening) return c;
          return { ...c, durationSec: newDuration, listening: newListening };
        }),
      );
      setListeningVersion((v) => v + 1);
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  // ----------------------------------------------------------------
  // 内部：发送 RWI 指令
  // ----------------------------------------------------------------
  const sendCommand = useCallback(
    (type: RwiCommandType, data: Record<string, unknown>): boolean => {
      const ws = wsRef.current;
      if (!ws || ws.readyState !== WebSocket.OPEN) {
        message.warning('WebSocket 未连接，无法发送指令');
        return false;
      }
      const payload: RwiMessage = {
        id: genUuid(),
        version: '1.0',
        command: type,
        data,
      };
      try {
        ws.send(JSON.stringify(payload));
        return true;
      } catch (e) {
        message.error('发送指令失败：' + (e instanceof Error ? e.message : String(e)));
        return false;
      }
    },
    [],
  );

  // ----------------------------------------------------------------
  // 对外：重连入口
  // ----------------------------------------------------------------
  const reconnect = useCallback(() => {
    reconnectAttemptRef.current = 0;
    connect();
  }, [connect]);

  // ----------------------------------------------------------------
  // 对外：指令 handler
  // ----------------------------------------------------------------
  const handleBargeIn = useCallback(
    (callId: string) => {
      if (!callId) return;
      const ok = sendCommand('barge_in', {
        call_id: callId,
        mode: 'listen_and_speak',
        target_leg: 'a_leg',
      });
      if (ok) {
        setCalls((prev) =>
          prev.map((c) =>
            c.callId === callId ? appendSystemTranscript(c, '已发送强插指令', true) : c,
          ),
        );
        message.warning(`已对通话 ${callId} 触发强插`);
      }
    },
    [sendCommand],
  );

  const handleSpeakSubmit = useCallback(
    (callId: string, text: string) => {
      if (!callId || !text.trim()) return;
      const ok = sendCommand('speak', {
        call_id: callId,
        text: text.trim(),
        voice: 'default',
        speed: 1.0,
      });
      if (ok) {
        setCalls((prev) =>
          prev.map((c) =>
            c.callId === callId ? appendSystemTranscript(c, `[文本播报]: ${text.trim()}`) : c,
          ),
        );
        message.success(`已向通话 ${callId} 注入文本播报指令`);
      }
    },
    [sendCommand],
  );

  const handleToggleListen = useCallback(
    (callId: string) => {
      if (!callId) return;
      const next = !listeningSetRef.current.has(callId);
      const ok = sendCommand('listen', {
        call_id: callId,
        stream_url: '',
        format: 'pcmu',
      });
      if (ok) {
        if (next) {
          listeningSetRef.current.add(callId);
          message.info(`已开启通话 ${callId} 实时监听`);
        } else {
          listeningSetRef.current.delete(callId);
          message.info(`已关闭通话 ${callId} 监听`);
        }
        // 同步到 calls state 以立即反映 UI
        setCalls((prev) => prev.map((c) => (c.callId === callId ? { ...c, listening: next } : c)));
      }
    },
    [sendCommand],
  );

  const handleTransferSubmit = useCallback(
    (callId: string, target: string) => {
      if (!callId || !target.trim()) return;
      const ok = sendCommand('transfer', {
        call_id: callId,
        target: target.trim(),
        transfer_type: 'blind',
      });
      if (ok) {
        setCalls((prev) =>
          prev.map((c) =>
            c.callId === callId ? appendSystemTranscript(c, `已发起转接 → ${target.trim()}`) : c,
          ),
        );
        message.success(`已发送转接指令至目标: ${target.trim()}`);
      }
    },
    [sendCommand],
  );

  const handleHangup = useCallback(
    (callId: string) => {
      if (!callId) return;
      const ok = sendCommand('hangup', {
        call_id: callId,
        reason_code: 16,
      });
      if (ok) {
        setCalls((prev) =>
          prev.map((c) =>
            c.callId === callId ? appendSystemTranscript(c, '已发送挂断指令（原因码：16）') : c,
          ),
        );
        message.success(`通话 ${callId} 挂断指令已发送`);
      }
    },
    [sendCommand],
  );

  // useMemo 防止每次渲染返回新数组（calls 已是 state，引用稳定）
  const stableCalls = useMemo(() => calls, [calls]);

  return {
    calls: stableCalls,
    wsState,
    pingMs,
    reconnect,
    handleBargeIn,
    handleSpeakSubmit,
    handleToggleListen,
    handleTransferSubmit,
    handleHangup,
  };
}
