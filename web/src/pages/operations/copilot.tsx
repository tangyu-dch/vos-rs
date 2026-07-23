import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Input, ScrollShadow, Spinner } from '@heroui/react';
import {
  Send, Bot, User, AlertTriangle, Lightbulb, RefreshCw,
  MessageSquare, Download, Trash2, Square,
} from 'lucide-react';
import { api } from '@/services/client';
import { getAccessToken } from '@/services/auth';
import { message } from '@/utils/toast';
import { SessionSidebar } from './copilot-sidebar';
import {
  CopilotMessageDTO, CopilotSession, DegradedBanner, LlmStateChip, ActiveModelBadge,
  MarkdownReport, MessageItem, StreamingIndicator, buildExportMarkdown,
  parseLlmError, parseLlmState, streamChat, toMessageItem,
} from './copilot-shared';

// 预设查询：对齐后端真实能力（分析/建议/可视化），不夸大为"下发/执行"
const PRESETS = [
  { title: '排查 13800138000 挂断原因', desc: '排查 13800138000 为什么在 10:15 被挂断' },
  { title: '生成最新呼叫 SIP 梯形图', desc: '生成最新呼叫的完整 SIP Ladder Diagram 梯形图' },
  { title: '查询 AI 伪造声音拦截记录', desc: '查询近期 AI 伪造声音拦截日志与拦截记录' },
  { title: '评估当前 CPS 与丢包率', desc: '评估当前 CPS、RTP 丢包率与集群节点探活状态' },
  { title: '分析网关路由配置', desc: '分析当前网关路由配置并给出高可用建议' },
  { title: '分析 DID 号码的日常使用成本', desc: '分析当前系统中 DID 号码的日常租用与呼出成本' },
  { title: '检查计费余额预警限额', desc: '查询计费账户余额与防欠费熔断配置' },
  { title: '审计 SIP 注册异常', desc: '审计并分析近期分机与外部中继注册失败的原因' },
];

const WELCOME_TEXT = '您好！我是 vos-rs 电信级 LLM 智能运维 Copilot。我可以基于**真实业务数据**为您：分析 SIP 抓包、绘制信令交互梯形图 (Call Ladder Diagram)、识别 QoS 异常并提供运维建议。\n\n下方是常用场景，也可直接在输入框描述您的问题。';

interface SessionListResponse { sessions: CopilotSession[]; }
interface SessionDetailResponse { session: CopilotSession; messages: CopilotMessageDTO[]; }

export function CopilotPage() {
  const [sessions, setSessions] = useState<CopilotSession[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MessageItem[]>([]);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [loadingSessions, setLoadingSessions] = useState(false);
  const [loadingMessages, setLoadingMessages] = useState(false);
  const [sending, setSending] = useState(false);
  const [inputQuery, setInputQuery] = useState('');
  const abortRef = useRef<AbortController | null>(null);

  // ============ 自动滚动到底部（流式输出 + 新消息）============
  const scrollRef = useRef<HTMLDivElement>(null);
  // 用户是否贴底（接近底部）：贴底时自动滚，上滑阅读时不打断
  const pinnedToBottomRef = useRef(true);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    // 距底部 80px 以内视为贴底
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    pinnedToBottomRef.current = distance < 80;
  }, []);

  // 消息变化或发送状态变化时，若贴底则平滑滚动到底部
  useEffect(() => {
    if (!pinnedToBottomRef.current) return;
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
  }, [messages, sending]);

  // 切换会话时重置贴底状态并立即滚动
  useEffect(() => {
    pinnedToBottomRef.current = true;
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight });
  }, [currentId]);

  // ============ 中断当前流 ============
  const abortStream = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
  }, []);

  // ============ 会话列表加载 ============
  const refreshSessions = useCallback(async () => {
    setLoadingSessions(true);
    try {
      const res = await api.get<SessionListResponse>('/copilot/sessions', { limit: 50 });
      setSessions(res.sessions || []);
    } catch {
      message.error('加载会话列表失败');
    } finally {
      setLoadingSessions(false);
    }
  }, []);

  useEffect(() => { refreshSessions(); }, [refreshSessions]);

  // ============ 选中会话 → 中断流 + 加载消息 ============
  const loadSession = useCallback(async (id: string) => {
    abortStream();
    setLoadingMessages(true);
    setCurrentId(id);
    try {
      const res = await api.get<SessionDetailResponse>(`/copilot/sessions/${id}`);
      setMessages(res.messages.map(toMessageItem));
    } catch {
      message.error('加载会话消息失败');
      setMessages([]);
    } finally {
      setLoadingMessages(false);
    }
  }, [abortStream]);

  // ============ 新建会话（检查未开始的对话，避免重复创建空会话）============
  const handleCreate = useCallback(async () => {
    abortStream();
    // 先在本地找未开始的对话（message_count === 0）
    const emptySession = sessions.find((s) => s.message_count === 0);
    if (emptySession) {
      setCurrentId(emptySession.id);
      setMessages([]);
      message.info('已切换到未开始的对话');
      return;
    }
    // 没有空会话，才创建新的
    try {
      const session = await api.post<CopilotSession>('/copilot/sessions', {});
      setSessions((prev) => [session, ...prev]);
      setCurrentId(session.id);
      setMessages([]);
      message.success('已创建新对话');
    } catch {
      message.error('创建会话失败');
    }
  }, [abortStream, sessions]);

  // ============ 删除会话 ============
  const handleDelete = useCallback(async (id: string) => {
    if (!window.confirm('确认删除该会话？所有消息将一并删除。')) return;
    try {
      await api.delete(`/copilot/sessions/${id}`);
      setSessions((prev) => prev.filter((s) => s.id !== id));
      if (currentId === id) {
        setCurrentId(null);
        setMessages([]);
      }
      message.success('会话已删除');
    } catch {
      message.error('删除会话失败');
    }
  }, [currentId]);

  // ============ 置顶/取消置顶 ============
  const handleTogglePin = useCallback(async (id: string, pinned: boolean) => {
    try {
      const updated = await api.put<CopilotSession>(`/copilot/sessions/${id}`, { pinned });
      setSessions((prev) => prev.map((s) => (s.id === id ? updated : s)));
    } catch {
      message.error('更新置顶状态失败');
    }
  }, []);

  // ============ 发送消息（SSE 流式 + 打字机渲染）============
  const handleSend = useCallback(async (queryText?: string) => {
    const query = (queryText || inputQuery).trim();
    if (!query || sending) return;

    // 中断上一个流
    abortStream();
    const controller = new AbortController();
    abortRef.current = controller;

    // 若无当前会话，先复用空会话或创建
    let sessionId = currentId;
    if (!sessionId) {
      const emptySession = sessions.find((s) => s.message_count === 0);
      if (emptySession) {
        sessionId = emptySession.id;
        setCurrentId(emptySession.id);
      } else {
        try {
          const session = await api.post<CopilotSession>('/copilot/sessions', {});
          sessionId = session.id;
          setSessions((prev) => [session, ...prev]);
          setCurrentId(session.id);
        } catch {
          message.error('创建会话失败');
          return;
        }
      }
    }

    // 乐观追加用户消息 + 空 bot 消息占位（流式逐字填充）
    const userTempId = `tmp-user-${Date.now()}`;
    const botTempId = `tmp-bot-${Date.now()}`;
    const ts = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    const userMsg: MessageItem = { id: userTempId, sender: 'user', text: query, timestamp: ts };
    const botMsg: MessageItem = { id: botTempId, sender: 'bot', text: '', timestamp: ts };
    setMessages((prev) => [...prev, userMsg, botMsg]);
    if (!queryText) setInputQuery('');
    setSending(true);

    const token = getAccessToken();
    if (!token) {
      message.error('登录已失效，请重新登录');
      setSending(false);
      return;
    }
    const url = `/api/v1/copilot/sessions/${sessionId}/chat/stream`;

    try {
      await streamChat(url, token, query, {
        onUserMessage: (msg) => {
          setMessages((prev) => prev.map((m) => (m.id === userTempId ? toMessageItem(msg) : m)));
        },
        onContext: (ctx) => {
          // LLM 状态信息（梯形图已内嵌到 LLM 回答的 markdown 中，不再单独推送）
          setMessages((prev) => prev.map((m) => (m.id === botTempId ? {
            ...m,
            llmEnabled: ctx.llm_enabled,
            llmStatus: ctx.llm_status,
            intent: ctx.intent,
          } : m)));
        },
        onDelta: (text) => {
          // 逐字追加（打字机效果）
          setMessages((prev) => prev.map((m) => (m.id === botTempId ? { ...m, text: m.text + text } : m)));
        },
        onDone: (data) => {
          // 用后端正式 assistant 消息替换临时占位
          setMessages((prev) => prev.map((m) => (m.id === botTempId ? toMessageItem(data.assistant_message) : m)));
          setSessions((prev) => {
            const others = prev.filter((s) => s.id !== data.session.id);
            return [data.session, ...others];
          });
        },
        onError: (error) => {
          setMessages((prev) => prev.map((m) => (m.id === botTempId ? {
            ...m,
            text: m.text || `诊断失败：${error}`,
            llmEnabled: false,
            llmStatus: m.llmStatus || '调用失败',
          } : m)));
        },
      }, controller.signal);
    } catch (err) {
      // 用户主动中断（abort）时保留已有内容，不清空
      if (!controller.signal.aborted) {
        const errorText = err instanceof Error ? err.message : String(err);
        setMessages((prev) => prev.map((m) => (m.id === botTempId ? {
          ...m,
          text: m.text || `诊断失败：${errorText}`,
          llmEnabled: false,
          llmStatus: '调用失败',
        } : m)));
      }
    } finally {
      setSending(false);
      if (abortRef.current === controller) abortRef.current = null;
    }
  }, [abortStream, currentId, inputQuery, sending, sessions]);

  // ============ 导出报告 ============
  const handleExport = useCallback(() => {
    if (messages.length === 0) {
      message.warning('没有可导出的诊断记录');
      return;
    }
    const md = buildExportMarkdown(messages);
    const blob = new Blob(['\ufeff' + md], { type: 'text/markdown;charset=utf-8;' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.setAttribute('href', url);
    link.setAttribute('download', `Copilot_Diagnosis_Report_${new Date().toISOString().slice(0, 10)}.md`);
    link.style.visibility = 'hidden';
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    message.success('已导出 Copilot 诊断分析报告 (Markdown)');
  }, [messages]);

  const hasMessages = messages.length > 0;
  // 欢迎页：无消息且不在加载/发送中时显示（新建会话后也会显示）
  const showWelcome = !hasMessages && !loadingMessages && !sending;

  return (
    <div className="h-[calc(100vh-100px)] flex flex-row relative bg-transparent">
      {/* 左侧：会话列表侧栏 */}
      <SessionSidebar
        sessions={sessions}
        currentId={currentId}
        loading={loadingSessions}
        collapsed={sidebarCollapsed}
        onSelect={(id) => { if (id !== currentId) loadSession(id); }}
        onCreate={handleCreate}
        onDelete={handleDelete}
        onTogglePin={handleTogglePin}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
      />

      {/* 右侧：主聊天区 */}
      <div className="flex-1 flex flex-col min-w-0 relative">
        {/* 顶部悬浮操作按钮 */}
        <div className="absolute top-0 right-4 z-10 flex gap-2 items-center">
          <ActiveModelBadge />
          {(hasMessages || sending) && (
            <>
              {sending && (
                <Button
                  size="sm"
                  color="danger"
                  variant="flat"
                  onPress={abortStream}
                  startContent={<Square className="w-3 h-3" />}
                >
                  停止生成
                </Button>
              )}
              <Button
                size="sm"
                variant="flat"
                onPress={handleExport}
                isDisabled={sending}
                startContent={<Download className="w-3.5 h-3.5" />}
              >
                导出报告
              </Button>
              <Button
                size="sm"
                variant="flat"
                color="primary"
                startContent={<RefreshCw className="w-3.5 h-3.5" />}
                onPress={handleCreate}
              >
                新对话
              </Button>
            </>
          )}
        </div>

        {/* 主沉浸聊天区 */}
        <div className="flex-1 flex flex-col min-h-0 justify-between items-center w-full">
          <ScrollShadow ref={scrollRef} onScroll={handleScroll} className="w-full flex-1 px-4 py-6 space-y-6 overflow-y-auto min-h-0">
            <div className="max-w-5xl mx-auto w-full space-y-6">
              {/* 欢迎页（无消息时显示，包括新建会话后）*/}
              {showWelcome && (
                <div className="flex flex-col items-center justify-center py-12 w-full">
                  <div className="w-14 h-14 rounded-2xl bg-primary/15 border border-primary/30 flex items-center justify-center text-primary mb-6 shadow-sm">
                    <Bot className="w-7 h-7" />
                  </div>
                  <h1 className="text-xl font-bold text-foreground text-center mb-8">
                    有什么我能帮你的吗？
                  </h1>
                  <div className="max-w-2xl mx-auto mb-6">
                    <MarkdownReport content={WELCOME_TEXT} />
                  </div>
                  <div className="flex flex-wrap gap-2.5 justify-center max-w-2xl mx-auto">
                    {PRESETS.map((p, idx) => (
                      <button
                        key={idx}
                        onClick={() => handleSend(p.desc)}
                        className="px-4 py-2 text-xs rounded-full border border-default-200 hover:border-primary hover:bg-primary/10 text-default-600 hover:text-primary transition-all duration-200 shadow-sm font-medium"
                      >
                        {p.title}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* 加载消息中 */}
              {loadingMessages && (
                <div className="flex items-center justify-center py-12">
                  <Spinner size="lg" />
                </div>
              )}

              {/* 对话消息展示 */}
              {!loadingMessages && hasMessages && messages.map((m) => {
                const llmState = parseLlmState(m.llmStatus, m.llmEnabled);
                const llmError = llmState === 'degraded' ? parseLlmError(m.llmStatus) : '';
                const isStreaming = m.sender === 'bot' && m.text === '' && sending;
                return (
                  <div
                    key={m.id}
                    className={`flex gap-4 w-full ${m.sender === 'user' ? 'ml-auto flex-row-reverse max-w-2xl' : 'max-w-4xl'}`}
                  >
                    {/* 头像 */}
                    <div
                      className={`w-10 h-10 rounded-2xl flex items-center justify-center shrink-0 font-bold shadow-sm ${
                        m.sender === 'user'
                          ? 'bg-primary text-primary-foreground'
                          : 'bg-primary/10 border border-primary/30 text-primary'
                      }`}
                    >
                      {m.sender === 'user' ? <User className="w-5 h-5" /> : <Bot className="w-5 h-5" />}
                    </div>
                    <div className="flex flex-col gap-2.5 flex-1 min-w-0">
                      {/* 消息气泡 */}
                      <div
                        className={`p-4 rounded-2xl text-xs leading-relaxed shadow-sm ${
                          m.sender === 'user'
                            ? 'bg-primary text-primary-foreground rounded-tr-none'
                            : 'bg-content1 text-foreground rounded-tl-none border border-default-200'
                        }`}
                      >
                        <div className={`flex items-center justify-between text-[10px] mb-1.5 font-mono gap-2 ${
                          m.sender === 'user' ? 'text-primary-foreground/70' : 'text-default-400'
                        }`}>
                          <span className="flex items-center gap-2">
                            <span>{m.sender === 'user' ? 'OPERATOR' : 'COPILOT'}</span>
                            {m.sender === 'bot' && <LlmStateChip state={llmState} status={m.llmStatus} />}
                          </span>
                          <span>{m.timestamp}</span>
                        </div>
                        {m.sender === 'user' ? (
                          <p className="whitespace-pre-wrap font-medium text-xs">{m.text}</p>
                        ) : isStreaming ? (
                          <StreamingIndicator />
                        ) : (
                          <MarkdownReport content={m.text} />
                        )}
                        {llmState === 'degraded' && <DegradedBanner error={llmError} />}
                      </div>

                      {/* 根因分析卡片（warning 主题，加深对比度）*/}
                      {m.rootCause && (
                        <div className="p-3.5 bg-warning/10 border border-warning/30 rounded-xl text-xs flex flex-col gap-1.5 shadow-sm">
                          <div className="flex items-center gap-1.5 text-warning font-bold">
                            <AlertTriangle className="w-4 h-4" />
                            <span>根因分析 (Root Cause)</span>
                          </div>
                          <div className="text-foreground text-[11px] pl-5 leading-relaxed">
                            <MarkdownReport content={m.rootCause} />
                          </div>
                        </div>
                      )}

                      {/* 建议动作卡片（primary 主题，加深对比度）*/}
                      {m.suggestedAction && (
                        <div className="p-3.5 bg-primary/10 border border-primary/30 rounded-xl text-xs flex flex-col gap-1.5 shadow-sm">
                          <div className="flex items-center gap-1.5 text-primary font-bold">
                            <Lightbulb className="w-4 h-4" />
                            <span>建议动作 (Suggested Action)</span>
                          </div>
                          <div className="text-foreground text-[11px] pl-5 leading-relaxed">
                            <MarkdownReport content={m.suggestedAction} />
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </ScrollShadow>

          {/* 底部浮动输入框胶囊（焦点态增强）*/}
          <div className="w-full px-4 py-4 shrink-0 bg-transparent">
            <div className="w-full max-w-5xl mx-auto rounded-3xl border-2 border-default-200 hover:border-primary/40 focus-within:border-primary focus-within:ring-4 focus-within:ring-primary/10 bg-content1 shadow-lg p-2 flex items-center gap-2 transition-all duration-200">
              <Input
                variant="flat"
                classNames={{
                  inputWrapper: 'bg-transparent shadow-none hover:bg-transparent focus-within:bg-transparent',
                  input: 'text-sm',
                }}
                placeholder="询问 Copilot 排查通话问题 (如: 查一下刚才 13800138000 为什么被断开...)"
                value={inputQuery}
                onValueChange={setInputQuery}
                onKeyDown={(e) => e.key === 'Enter' && !e.shiftKey && (e.preventDefault(), handleSend())}
                isDisabled={sending}
                startContent={<MessageSquare className="w-4 h-4 text-default-400" />}
                endContent={
                  <Button
                    size="sm"
                    color="primary"
                    className="rounded-2xl px-4 text-primary-foreground font-bold"
                    isLoading={sending}
                    onPress={() => handleSend()}
                    startContent={!sending && <Send className="w-3.5 h-3.5" />}
                  >
                    发送
                  </Button>
                }
              />
            </div>
          </div>
        </div>
      </div>

      {/* 删除会话的浮动提示（无障碍） */}
      <span className="sr-only">
        <Trash2 /> 删除会话
      </span>
    </div>
  );
}
