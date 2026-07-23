//! Copilot 共享类型、helper 与子组件
//!
//! 拆分自 copilot.tsx，避免主页面文件超过 500 行。
//! 本文件不含任何业务状态，纯展示/解析工具。

import { useEffect, useState } from 'react';
import { Chip } from '@heroui/react';
import {
  AlertTriangle, ChevronDown, Cpu, Info, Settings2,
} from 'lucide-react';
import { Link } from 'react-router-dom';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { api } from '@/services/client';

// ============ 类型定义 ============

/** 聊天 UI 内部消息表示（兼容后端历史消息 + 临时未落库消息） */
export interface MessageItem {
  id: string;
  sender: 'user' | 'bot';
  text: string;
  rootCause?: string;
  suggestedAction?: string;
  ladderAscii?: string;
  llmEnabled?: boolean;
  llmStatus?: string;
  intent?: string;
  timestamp: string;
}

/** Copilot 会话元数据（对齐后端 CopilotSession 结构） */
export interface CopilotSession {
  id: string;
  title: string;
  operator: string;
  llm_provider: string | null;
  llm_model: string | null;
  pinned: boolean;
  archived: boolean;
  message_count: number;
  last_message_at: string | null;
  created_at: string;
  updated_at: string;
}

/** Copilot 单条消息 DTO（对齐后端 CopilotMessage 结构） */
export interface CopilotMessageDTO {
  id: number;
  session_id: string;
  role: string;
  content: string;
  root_cause: string | null;
  suggested_action: string | null;
  ladder_diagram_ascii: string | null;
  llm_enabled: boolean | null;
  llm_status: string | null;
  intent: string | null;
  created_at: string;
}

// ============ LLM 状态解析 ============

export type LlmState = 'active' | 'degraded' | 'unconfigured';

export function parseLlmState(status?: string, enabled?: boolean): LlmState {
  if (!enabled) return 'unconfigured';
  if (!status) return 'unconfigured';
  if (status.includes('调用失败')) return 'degraded';
  if (status.includes('未配置')) return 'unconfigured';
  return 'active';
}

export function parseLlmMeta(status?: string): string {
  if (!status) return '';
  const m = status.match(/provider=([^,)]+),\s*model=([^)]+)/);
  return m ? `${m[1]} · ${m[2]}` : '';
}

export function parseLlmError(status?: string): string {
  if (!status) return '';
  const m = status.match(/调用失败[:：]\s*([^；]+)/);
  return m ? m[1] : '';
}

// ============ SIP 梯形图着色 ============

export function ladderLineClass(line: string): string {
  if (/^\s*\[.*\]\s*\[.*\]\s*\[.*\]\s*$/.test(line)) {
    return 'text-primary font-bold';
  }
  if (/^\s*\+\d+ms/.test(line)) return 'text-default-400';
  if (/^\s*\|[\s|]*\|\s*$/.test(line) || line.trim() === '') {
    return 'text-default-300';
  }
  if (/[456]\d{2}\s/.test(line) || /SIP\s+[456]\d{2}/.test(line)) {
    return 'text-danger font-semibold';
  }
  if (/BYE/.test(line)) return 'text-warning font-semibold';
  if (/200\s*OK/.test(line)) return 'text-success font-semibold';
  if (/INVITE/.test(line)) return 'text-primary font-semibold';
  if (/100\s*Trying|180\s*Ringing|183\s*Session/.test(line)) {
    return 'text-default-500';
  }
  if (/CANCEL/.test(line)) return 'text-warning';
  return 'text-foreground';
}

// ============ 共享子组件 ============

export function LlmStateChip({ state, status }: { state: LlmState; status?: string }) {
  if (state === 'active') {
    const meta = parseLlmMeta(status);
    return (
      <Chip
        size="sm"
        color="primary"
        variant="flat"
        className="text-[10px] h-5"
        startContent={<Cpu className="w-2.5 h-2.5" />}
      >
        {meta ? `LLM · ${meta}` : 'LLM 已启用'}
      </Chip>
    );
  }
  if (state === 'degraded') {
    return (
      <Chip
        size="sm"
        color="warning"
        variant="flat"
        className="text-[10px] h-5"
        startContent={<AlertTriangle className="w-2.5 h-2.5" />}
      >
        LLM 降级 · 结构化数据
      </Chip>
    );
  }
  return (
    <Chip
      size="sm"
      variant="flat"
      className="text-[10px] h-5 text-default-500"
      startContent={<Info className="w-2.5 h-2.5" />}
    >
      未配置 LLM · 结构化数据
    </Chip>
  );
}

export function DegradedBanner({ error }: { error: string }) {
  const [expanded, setExpanded] = useState(false);
  if (!error) return null;
  return (
    <div className="mt-1.5">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-1.5 text-[10px] text-warning hover:text-warning-600 transition-colors"
      >
        <ChevronDown className={`w-3 h-3 transition-transform ${expanded ? 'rotate-180' : ''}`} />
        <span>查看 LLM 调用失败详情</span>
      </button>
      {expanded && (
        <div className="mt-1 p-2 rounded-lg bg-warning/5 border border-warning/20 text-[10px] text-default-600 font-mono break-all">
          {error}
        </div>
      )}
    </div>
  );
}

/** Markdown 渲染：手写轻量 components，避免引入 @tailwindcss/typography */
export function MarkdownReport({ content }: { content: string }) {
  return (
    <div className="text-xs">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          h1: ({ children }) => <h1 className="text-sm font-bold mt-2 mb-1.5 text-foreground">{children}</h1>,
          h2: ({ children }) => <h2 className="text-sm font-bold mt-2 mb-1.5 text-primary">{children}</h2>,
          h3: ({ children }) => <h3 className="text-[13px] font-semibold mt-1.5 mb-1 text-foreground">{children}</h3>,
          h4: ({ children }) => <h4 className="text-xs font-semibold mt-1.5 mb-0.5 text-foreground">{children}</h4>,
          p: ({ children }) => <p className="my-1 leading-relaxed">{children}</p>,
          ul: ({ children }) => <ul className="my-1 pl-4 space-y-0.5 list-disc marker:text-primary/50">{children}</ul>,
          ol: ({ children }) => <ol className="my-1 pl-4 space-y-0.5 list-decimal marker:text-primary/50">{children}</ol>,
          li: ({ children }) => <li className="leading-relaxed pl-0.5">{children}</li>,
          strong: ({ children }) => <strong className="font-bold text-primary">{children}</strong>,
          em: ({ children }) => <em className="italic text-default-600">{children}</em>,
          code: ({ className, children }) => {
            const isBlock = className?.includes('language-');
            if (isBlock) {
              return <code className={`${className} text-[11px] font-mono text-success`}>{children}</code>;
            }
            return <code className="px-1 py-0.5 rounded bg-content2 text-primary text-[11px] font-mono">{children}</code>;
          },
          pre: ({ children }) => <pre className="my-2 p-2.5 rounded-lg bg-content2 overflow-x-auto border border-default-200">{children}</pre>,
          blockquote: ({ children }) => (
            <blockquote className="my-1.5 pl-3 border-l-2 border-primary/40 text-default-600 italic">{children}</blockquote>
          ),
          hr: () => <hr className="my-2 border-default-200" />,
          table: ({ children }) => <table className="my-2 w-full text-[11px] border-collapse">{children}</table>,
          th: ({ children }) => <th className="border border-default-200 px-2 py-1 bg-content2 text-left font-semibold">{children}</th>,
          td: ({ children }) => <td className="border border-default-200 px-2 py-1">{children}</td>,
          a: ({ children, href }) => <a href={href} target="_blank" rel="noreferrer" className="text-primary underline hover:opacity-80">{children}</a>,
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}

// ============ 时间工具 ============

/** 把 ISO 时间字符串格式化为相对时间（如 "3 分钟前"、"刚刚"） */
export function timeAgo(iso?: string | null): string {
  if (!iso) return '';
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return '';
  const diff = Date.now() - t;
  if (diff < 60_000) return '刚刚';
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  if (diff < 7 * 86_400_000) return `${Math.floor(diff / 86_400_000)} 天前`;
  // 超过一周显示日期
  return new Date(iso).toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit' });
}

/** 把后端历史消息 DTO 转换为 UI MessageItem */
export function toMessageItem(m: CopilotMessageDTO): MessageItem {
  const isUser = m.role === 'user';
  return {
    id: `db-${m.id}`,
    sender: isUser ? 'user' : 'bot',
    text: m.content,
    rootCause: m.root_cause ?? undefined,
    suggestedAction: m.suggested_action ?? undefined,
    ladderAscii: m.ladder_diagram_ascii ?? undefined,
    llmEnabled: m.llm_enabled ?? undefined,
    llmStatus: m.llm_status ?? undefined,
    intent: m.intent ?? undefined,
    timestamp: new Date(m.created_at).toLocaleTimeString('zh-CN', { hour12: false }),
  };
}

// ============ SSE 流式处理 ============

/** SSE context 事件载荷 */
export interface StreamContext {
  intent: string;
  llm_enabled: boolean;
  llm_status: string;
}

/** SSE done 事件载荷 */
export interface StreamDone {
  session: CopilotSession;
  assistant_message: CopilotMessageDTO;
}

/** SSE 流式回调 */
export interface StreamCallbacks {
  onUserMessage: (msg: CopilotMessageDTO) => void;
  onContext: (ctx: StreamContext) => void;
  onDelta: (text: string) => void;
  onDone: (data: StreamDone) => void;
  onError: (error: string) => void;
}

/**
 * 调用 SSE 流式端点，逐事件回调。
 *
 * SSE 事件格式：`event: <name>\ndata: <json>\n\n`
 * 事件类型：user_message / context / delta / done / error
 */
export async function streamChat(
  url: string,
  token: string,
  query: string,
  callbacks: StreamCallbacks,
  signal?: AbortSignal,
): Promise<void> {
  const response = await fetch(url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ query }),
    signal,
  });

  if (!response.ok) {
    const text = await response.text().catch(() => '');
    throw new Error(`HTTP ${response.status}: ${text.slice(0, 200)}`);
  }

  const reader = response.body?.getReader();
  if (!reader) throw new Error('无法获取 SSE 流');

  const decoder = new TextDecoder();
  let buffer = '';
  let streamFinished = false;

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });

      // SSE 事件以双换行分隔
      const parts = buffer.split('\n\n');
      buffer = parts.pop() ?? '';

      for (const part of parts) {
        const lines = part.split('\n');
        let eventType = '';
        let eventData = '';
        for (const line of lines) {
          if (line.startsWith('event: ')) eventType = line.slice(7).trim();
          if (line.startsWith('data: ')) eventData = line.slice(6);
        }
        if (!eventType || !eventData) continue;

        try {
          const data = JSON.parse(eventData);
          switch (eventType) {
            case 'user_message': callbacks.onUserMessage(data); break;
            case 'context': callbacks.onContext(data); break;
            case 'delta': callbacks.onDelta(data.text ?? ''); break;
            case 'done': callbacks.onDone(data); streamFinished = true; break;
            case 'error': callbacks.onError(data.error ?? '未知错误'); streamFinished = true; break;
          }
        } catch {
          // 忽略解析失败的事件
        }
      }
      // 收到 done/error 事件后主动结束（后端 KeepAlive 可能不关闭连接）
      if (streamFinished) break;
    }
  } finally {
    // 主动关闭 reader，释放底层连接（防止 KeepAlive 导致连接挂起）
    await reader.cancel().catch(() => {});
  }
}

// ============ 流式加载指示器 ============

/** 流式生成中的加载指示器（三点跳动 + 文案） */
export function StreamingIndicator({ text = '正在思考...' }: { text?: string }) {
  return (
    <div className="flex items-center gap-1.5 py-1">
      <span className="w-1.5 h-1.5 rounded-full bg-primary animate-bounce" style={{ animationDelay: '0ms' }} />
      <span className="w-1.5 h-1.5 rounded-full bg-primary animate-bounce" style={{ animationDelay: '150ms' }} />
      <span className="w-1.5 h-1.5 rounded-full bg-primary animate-bounce" style={{ animationDelay: '300ms' }} />
      <span className="ml-1.5 text-[11px] text-default-400">{text}</span>
    </div>
  );
}

// ============ 报告导出 ============

/** 把当前会话消息列表构建为 Markdown 报告字符串 */
export function buildExportMarkdown(messages: MessageItem[]): string {
  let md = `# vos-rs Copilot 诊断分析报告\n导出时间: ${new Date().toLocaleString('zh-CN')}\n\n-------------------------------------------\n\n`;
  messages.forEach((m) => {
    const role = m.sender === 'user' ? 'OPERATOR (操作员)' : 'COPILOT (智能运维助手)';
    md += `### [${m.timestamp}] ${role}\n\n`;
    if (m.llmStatus) md += `> LLM 状态: ${m.llmStatus}\n\n`;
    md += `${m.text}\n\n`;
    if (m.rootCause) md += `> **根因分析 (Root Cause):**\n> ${m.rootCause}\n\n`;
    if (m.suggestedAction) md += `> **建议动作 (Suggested Action):**\n> ${m.suggestedAction}\n\n`;
    if (m.ladderAscii) md += `**SIP 信令交互梯形图:**\n\`\`\`text\n${m.ladderAscii}\n\`\`\`\n\n`;
    md += `-------------------------------------------\n\n`;
  });
  return md;
}

// ============ 当前启用模型徽标 ============

/** 顶部展示当前启用的 LLM 模型，点击跳转 /settings/llm 配置页 */
export function ActiveModelBadge() {
  const [model, setModel] = useState<string>('');

  useEffect(() => {
    let cancelled = false;
    api.get<{ provider: string; model: string } | null>('/llm-configs/active')
      .then((rec) => {
        if (!cancelled && rec) setModel(`${rec.provider} · ${rec.model}`);
      })
      .catch(() => { if (!cancelled) setModel(''); });
    return () => { cancelled = true; };
  }, []);

  return (
    <Link to="/settings/llm" className="inline-flex items-center no-underline">
      <Chip
        size="sm"
        variant="flat"
        color={model ? 'primary' : 'default'}
        className="text-[10px] h-5 cursor-pointer hover:opacity-80"
        startContent={<Cpu className="w-2.5 h-2.5" />}
        endContent={<Settings2 className="w-2.5 h-2.5 opacity-60" />}
      >
        {model || '未配置 LLM'}
      </Chip>
    </Link>
  );
}
