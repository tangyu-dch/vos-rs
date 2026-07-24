//! Copilot 主页面
//!
//! 职责：状态编排 + 业务逻辑（会话/消息/SSE/附件）+ 顶层布局骨架。
//! 渲染拆分至子组件：
//! - copilot-message.tsx: WelcomePanel / MessageBubble / MessagesLoading
//! - copilot-input.tsx: ComposerBar / AttachmentChips / AttachedFile 类型
//! - copilot-preview.tsx: ImageLightbox / CsvPreviewModal
//! - copilot-sidebar.tsx: SessionSidebar
//! - copilot-shared.tsx: 类型 / helper / SSE / MarkdownReport 等

import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Card, CardBody, ScrollShadow } from '@heroui/react';
import { Bot, Download, PanelLeft, Square, SquarePen, Trash2 } from 'lucide-react';
import { api } from '@/services/client';
import { PageHeader } from '@/components/detail-shell';
import { getAccessToken } from '@/services/auth';
import { message } from '@/utils/toast';
import { SessionSidebar } from './copilot-sidebar';
import {
  CopilotMessageDTO, CopilotSession, ActiveModelBadge,
  MessageItem, buildExportMarkdown, streamChat, toMessageItem,
} from './copilot-shared';
import {
  WelcomePanel, MessagesLoading, MessageBubble,
} from './copilot-message';
import {
  AttachedFile, AttachmentChips, ComposerBar,
} from './copilot-input';
import {
  ImageLightbox, CsvPreviewModal,
} from './copilot-preview';

interface SessionListResponse { sessions: CopilotSession[]; }
interface SessionDetailResponse { session: CopilotSession; messages: CopilotMessageDTO[]; }

export function CopilotPage() {
  // ============ 核心状态 ============
  const [sessions, setSessions] = useState<CopilotSession[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MessageItem[]>([]);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  // 小屏（< lg）下侧边栏默认隐藏，通过顶部按钮切换为浮层
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);
  const [loadingSessions, setLoadingSessions] = useState(false);
  const [loadingMessages, setLoadingMessages] = useState(false);
  const [sending, setSending] = useState(false);
  const [inputQuery, setInputQuery] = useState('');
  const abortRef = useRef<AbortController | null>(null);
  const [activeModel, setActiveModel] = useState<{ id: number; provider: string; model: string } | null>(null);

  // ============ 图片与 CSV 附件预览 Modal 状态 ============
  const [previewImage, setPreviewImage] = useState<string | null>(null);
  const [previewFile, setPreviewFile] = useState<{ name: string; content: string } | null>(null);

  // ============ 附件/图片上传状态 ============
  const [attachedFiles, setAttachedFiles] = useState<AttachedFile[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const processFiles = useCallback((files: FileList | File[]) => {
    Array.from(files).forEach((file) => {
      const isImage = file.type.startsWith('image/');
      const sizeStr = file.size < 1024 * 1024
        ? `${(file.size / 1024).toFixed(1)} KB`
        : `${(file.size / (1024 * 1024)).toFixed(1)} MB`;
      const id = `${Date.now()}-${Math.random().toString(36).substring(2, 7)}`;
      const reader = new FileReader();

      if (isImage) {
        reader.onload = (e) => {
          const base64 = e.target?.result as string;
          setAttachedFiles((prev) => [
            ...prev,
            { id, file, name: file.name, sizeStr, isImage: true, previewUrl: base64, base64Data: base64 },
          ]);
        };
        reader.readAsDataURL(file);
      } else {
        reader.onload = (e) => {
          const text = e.target?.result as string;
          setAttachedFiles((prev) => [
            ...prev,
            { id, file, name: file.name, sizeStr, isImage: false, textContent: text },
          ]);
        };
        reader.readAsText(file);
      }
    });
  }, []);

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    if (e.clipboardData.files && e.clipboardData.files.length > 0) {
      e.preventDefault();
      processFiles(e.clipboardData.files);
      message.success('已自动捕获剪贴板图片/文件附件');
    }
  }, [processFiles]);

  // ============ 自动滚动到底部（流式输出 + 新消息）============
  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedToBottomRef = useRef(true);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    // 距底部 80px 以内视为贴底
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    pinnedToBottomRef.current = distance < 80;
  }, []);

  useEffect(() => {
    if (!pinnedToBottomRef.current) return;
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
  }, [messages, sending]);

  useEffect(() => {
    pinnedToBottomRef.current = true;
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight });
  }, [currentId]);

  // ============ 获取当前启用的模型 ============
  const fetchActiveModel = useCallback(async () => {
    try {
      const rec = await api.get<{ id: number; provider: string; model: string } | null>('/llm-configs/active');
      setActiveModel(rec);
    } catch {
      setActiveModel(null);
    }
  }, []);

  useEffect(() => { fetchActiveModel(); }, [fetchActiveModel]);

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

  // ============ 新建会话（复用空会话，避免重复创建）============
  const handleCreate = useCallback(async () => {
    abortStream();
    const emptySession = sessions.find((s) => s.message_count === 0);
    if (emptySession) {
      setCurrentId(emptySession.id);
      setMessages([]);
      message.info('已切换到未开始的对话');
      return;
    }
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

  // ============ 复制报告到剪贴板 ============
  const handleCopyText = useCallback((text: string) => {
    navigator.clipboard.writeText(text);
    message.success('已复制分析报告至剪贴板');
  }, []);

  // ============ 发送消息（SSE 流式 + 打字机渲染）============
  const handleSend = useCallback(async (queryText?: string) => {
    const query = (queryText || inputQuery).trim();
    if ((!query && attachedFiles.length === 0) || sending) return;

    // 整合附件信息
    const images = attachedFiles.filter((f) => f.isImage && f.base64Data).map((f) => f.base64Data as string);
    const attachedTextFiles = attachedFiles
      .filter((f) => !f.isImage && f.textContent)
      .map((f) => ({ name: f.name, sizeStr: f.sizeStr, content: f.textContent }));
    const textAppend = attachedFiles
      .filter((f) => !f.isImage && f.textContent)
      .map((f) => `\n\n[📁 附加文件/数据: ${f.name}]\n\`\`\`\n${f.textContent}\n\`\`\``)
      .join('');
    const fullQuery = query + textAppend;
    const displayQuery = query || (attachedFiles.length > 0 ? `[发送了 ${attachedFiles.length} 个附件进行分析]` : '');

    setAttachedFiles([]);
    if (!queryText) setInputQuery('');

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

    // 乐观追加用户消息 + 空 bot 消息占位
    const userTempId = `tmp-user-${Date.now()}`;
    const botTempId = `tmp-bot-${Date.now()}`;
    const ts = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    const userMsg: MessageItem = {
      id: userTempId, sender: 'user', text: displayQuery,
      images: images.length > 0 ? images : undefined,
      files: attachedTextFiles.length > 0 ? attachedTextFiles : undefined,
      timestamp: ts,
    };
    const botMsg: MessageItem = { id: botTempId, sender: 'bot', text: '', timestamp: ts };
    setMessages((prev) => [...prev, userMsg, botMsg]);
    setSending(true);

    const token = getAccessToken();
    if (!token) {
      message.error('登录已失效，请重新登录');
      setSending(false);
      return;
    }

    // 发送时获取最新激活的模型配置
    let currentModelId = activeModel?.id;
    try {
      const rec = await api.get<{ id: number; provider: string; model: string } | null>('/llm-configs/active');
      if (rec) {
        setActiveModel(rec);
        currentModelId = rec.id;
      }
    } catch {}

    const url = `/api/v1/copilot/sessions/${sessionId}/chat/stream`;

    try {
      await streamChat(
        url, token, fullQuery,
        {
          onUserMessage: (msg) => {
            setMessages((prev) => prev.map((m) => (m.id === userTempId ? { ...toMessageItem(msg), images: m.images, files: m.files } : m)));
          },
          onContext: (ctx) => {
            setMessages((prev) => prev.map((m) => (m.id === botTempId ? {
              ...m, llmEnabled: ctx.llm_enabled, llmStatus: ctx.llm_status, intent: ctx.intent,
            } : m)));
          },
          onDelta: (text) => {
            setMessages((prev) => prev.map((m) => (m.id === botTempId ? { ...m, text: m.text + text } : m)));
          },
          onDone: (data) => {
            setMessages((prev) => prev.map((m) => (m.id === botTempId ? toMessageItem(data.assistant_message) : m)));
            setSessions((prev) => {
              const others = prev.filter((s) => s.id !== data.session.id);
              return [data.session, ...others];
            });
          },
          onError: (error) => {
            setMessages((prev) => prev.map((m) => (m.id === botTempId ? {
              ...m, text: m.text || `诊断失败：${error}`,
              llmEnabled: false, llmStatus: m.llmStatus || '调用失败',
            } : m)));
          },
        },
        currentModelId ?? undefined,
        controller.signal,
        images.length > 0 ? images : undefined,
      );
    } catch (err) {
      if (!controller.signal.aborted) {
        const errorText = err instanceof Error ? err.message : String(err);
        setMessages((prev) => prev.map((m) => (m.id === botTempId ? {
          ...m, text: m.text || `诊断失败：${errorText}`,
          llmEnabled: false, llmStatus: '调用失败',
        } : m)));
      }
    } finally {
      setSending(false);
      if (abortRef.current === controller) abortRef.current = null;
    }
  }, [abortStream, currentId, inputQuery, sending, sessions, attachedFiles, activeModel]);

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

  // ============ 渲染 ============
  const hasMessages = messages.length > 0;
  const showWelcome = !hasMessages && !loadingMessages && !sending;

  return (
    <div className="h-[calc(100vh-100px)] flex flex-row relative">
      {/* 小屏侧边栏浮层展开时的遮罩，点击关闭 */}
      {mobileSidebarOpen && (
        <div
          className="absolute inset-0 bg-black/40 z-30 lg:hidden"
          onClick={() => setMobileSidebarOpen(false)}
          aria-hidden="true"
        />
      )}

      {/* 左侧：会话列表侧栏（小屏默认隐藏，lg+ 始终展示） */}
      <SessionSidebar
        sessions={sessions}
        currentId={currentId}
        loading={loadingSessions}
        collapsed={sidebarCollapsed}
        mobileOpen={mobileSidebarOpen}
        onSelect={(id) => {
          if (id !== currentId) loadSession(id);
          setMobileSidebarOpen(false);
        }}
        onCreate={() => {
          handleCreate();
          setMobileSidebarOpen(false);
        }}
        onDelete={handleDelete}
        onTogglePin={handleTogglePin}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
      />

      {/* 右侧：主聊天区 */}
      <div className="flex-1 flex flex-col min-w-0 bg-content1">
        {/* 顶部固定标题与操作栏 */}
        <Card shadow="sm" className="p-2 shrink-0 rounded-none">
          <CardBody className="p-4">
            <PageHeader
              icon={Bot}
              title="Copilot 智能运维助手"
              subtitle="自然语言抓包排障 · SIP 梯形图自动合成"
              actions={
                <>
                  {/* 小屏：展开会话历史浮层 */}
                  <Button
                    isIconOnly
                    size="sm"
                    variant="flat"
                    className="lg:hidden"
                    onPress={() => setMobileSidebarOpen(true)}
                    aria-label="显示会话历史"
                  >
                    <PanelLeft className="w-4 h-4" />
                  </Button>
                  <ActiveModelBadge activeModel={activeModel} />
                  {(hasMessages || sending) && (
                    <>
                      {sending && (
                        <Button size="sm" color="danger" variant="flat" onPress={abortStream}
                          startContent={<Square className="w-4 h-4" />}>
                          停止生成
                        </Button>
                      )}
                      <Button size="sm" variant="flat" onPress={handleExport} isDisabled={sending}
                        startContent={<Download className="w-4 h-4" />}>
                        导出报告
                      </Button>
                      <Button size="sm" variant="flat" color="primary" onPress={handleCreate}
                        startContent={<SquarePen className="w-4 h-4" />}>
                        新对话
                      </Button>
                    </>
                  )}
                </>
              }
            />
          </CardBody>
        </Card>

        {/* 主沉浸聊天区 */}
        <div className="flex-1 flex flex-col min-h-0 justify-between items-center w-full border border-default-200/50 rounded-2xl overflow-hidden bg-content1">
          <ScrollShadow ref={scrollRef} onScroll={handleScroll} className="w-full flex-1 px-4 py-6 space-y-6 overflow-y-auto min-h-0">
            <div className="max-w-[94%] mx-auto w-full space-y-6">
              {showWelcome && <WelcomePanel onPresetClick={(desc) => handleSend(desc)} />}
              {loadingMessages && <MessagesLoading />}
              {!loadingMessages && hasMessages && messages.map((m) => (
                <MessageBubble
                  key={m.id}
                  message={m}
                  sending={sending}
                  onImageClick={setPreviewImage}
                  onFileClick={setPreviewFile}
                  onCopyText={handleCopyText}
                />
              ))}
            </div>
          </ScrollShadow>

          {/* 附件预览 Chips + 底部输入框 */}
          <AttachmentChips
            files={attachedFiles}
            onRemove={(id) => setAttachedFiles((prev) => prev.filter((f) => f.id !== id))}
            onPreviewImage={setPreviewImage}
          />
          <ComposerBar
            inputQuery={inputQuery}
            setInputQuery={setInputQuery}
            sending={sending}
            onSend={() => handleSend()}
            onPaste={handlePaste}
            fileInputRef={fileInputRef}
            onFileSelect={processFiles}
          />
        </div>
      </div>

      {/* 预览 Modal */}
      <ImageLightbox url={previewImage} onClose={() => setPreviewImage(null)} />
      <CsvPreviewModal file={previewFile} onClose={() => setPreviewFile(null)} />

      {/* 删除会话的浮动提示（无障碍） */}
      <span className="sr-only">
        <Trash2 /> 删除会话
      </span>
    </div>
  );
}
