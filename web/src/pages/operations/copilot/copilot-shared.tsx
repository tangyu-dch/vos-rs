//! Copilot 共享类型、helper 与子组件
//!
//! 拆分自 copilot.tsx，避免主页面文件超过 500 行。
//! 本文件不含任何业务状态，纯展示/解析工具。

import { useState } from 'react';
import { Chip, Dropdown, DropdownTrigger, DropdownMenu, DropdownItem } from '@heroui/react';
import { AlertTriangle, Check, ChevronDown, Cpu, Info, Sparkles } from 'lucide-react';
import { api } from '@/services/client';
import { message } from '@/utils/toast';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { findPreset, type LlmConfigRecord } from '../../settings/llm-presets';

// ============ 类型定义 ============

/** 聊天 UI 内部消息表示（兼容后端历史消息 + 临时未落库消息） */
export interface MessageItem {
  id: string;
  sender: 'user' | 'bot';
  text: string;
  images?: string[];
  files?: { name: string; sizeStr?: string; content?: string }[];
  rootCause?: string;
  suggestedAction?: string;
  ladderAscii?: string;
  llmEnabled?: boolean;
  llmStatus?: string;
  intent?: string;
  timestamp: string;
  /** 工具调用过程记录（流式期间通过 tool_start/tool_result 事件累积） */
  toolCalls?: ToolCallTrace[];
}

/** 单次工具调用轨迹：开始事件 + 结果事件 */
export interface ToolCallTrace {
  /** 工具名（如 vos_get_daily_report） */
  name: string;
  /** 工具参数（tool_start 事件的 args） */
  args?: unknown;
  /** 结果预览（tool_result 事件的 result_preview） */
  resultPreview?: string;
  /** 调用或审批状态。 */
  status:
    'pending' | 'done' | 'approval_required' | 'approving' | 'approved' | 'rejected' | 'failed';
  /** 写操作进入审批流程后由后端持久化的动作。 */
  action?: CopilotAction;
}

/** Copilot 写操作审批记录。 */
export interface CopilotAction {
  id: string;
  session_id: string;
  operator: string;
  requested_role: string;
  tool_name: string;
  tool_arguments: unknown;
  risk_level: 'write' | 'high_risk' | string;
  status: 'pending' | 'executing' | 'approved' | 'rejected' | 'failed' | string;
  reviewed_by?: string | null;
  reviewed_role?: string | null;
  review_note?: string | null;
  result?: unknown;
  created_at: string;
  reviewed_at?: string | null;
  completed_at?: string | null;
}

/** 工具名 → 中文标签映射表（用于 ToolCallCard 展示） */
export const TOOL_LABELS: Record<string, string> = {
  vos_get_dashboard_stats: '平台运行概览',
  vos_get_daily_report: '每日汇报聚合',
  vos_list_cdrs: '呼叫详单查询',
  vos_get_sip_flows: 'SIP 信令抓包',
  vos_list_active_calls: '实时并发通话',
  vos_terminate_call: '强制拆线挂断',
  vos_list_registrations: '分机注册状态',
  vos_list_gateways: '中继网关列表',
  vos_preview_route: '路由试算',
  vos_list_anti_fraud_rules: '风控规则列表',
  vos_list_extensions: '分机账号列表',
  vos_create_extension: '创建分机',
  vos_delete_extension: '删除分机',
  vos_list_ivr_menus: 'IVR 菜单列表',
  vos_create_ivr_menu: '创建 IVR 菜单',
  vos_add_ivr_node: '添加 IVR 节点',
  vos_delete_ivr_menu: '删除 IVR 菜单',
  vos_create_gateway: '创建网关',
  vos_delete_gateway: '删除网关',
  vos_create_route: '创建路由',
  vos_list_routes: '路由规则列表',
  vos_delete_route: '删除路由',
  vos_list_billing_accounts: '计费账户列表',
  vos_recharge_billing_account: '账户充值',
  vos_create_anti_fraud_rule: '创建风控规则',
  vos_delete_anti_fraud_rule: '删除风控规则',
  vos_export_cdrs: '导出呼叫详单',
  vos_export_extensions: '导出分机',
  vos_export_gateways: '导出网关',
  vos_export_routes: '导出路由',
  vos_export_billing_accounts: '导出计费账户',
  vos_import_extensions: '批量导入分机',
  vos_import_gateways: '批量导入网关',
  vos_import_routes: '批量导入路由',
};

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
  images?: string[] | null;
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
    <div className="text-[13px] leading-6 text-default-700">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          h1: ({ children }) => (
            <h1 className="text-base font-semibold mt-4 mb-2 pb-1.5 border-b border-default-200 text-foreground">
              {children}
            </h1>
          ),
          h2: ({ children }) => (
            <h2 className="text-sm font-semibold mt-4 mb-1.5 text-foreground">{children}</h2>
          ),
          h3: ({ children }) => (
            <h3 className="text-[13px] font-semibold mt-2 mb-1 text-foreground">{children}</h3>
          ),
          h4: ({ children }) => (
            <h4 className="text-xs font-semibold mt-2 mb-0.5 text-default-700">{children}</h4>
          ),
          p: ({ children }) => <p className="my-1.5 leading-6 text-default-700">{children}</p>,
          ul: ({ children }) => (
            <ul className="my-1.5 pl-5 space-y-1 list-disc marker:text-default-400">{children}</ul>
          ),
          ol: ({ children }) => (
            <ol className="my-1.5 pl-5 space-y-1 list-decimal marker:text-default-400">
              {children}
            </ol>
          ),
          li: ({ children }) => <li className="leading-relaxed pl-0.5">{children}</li>,
          strong: ({ children }) => (
            <strong className="font-semibold text-foreground">{children}</strong>
          ),
          em: ({ children }) => <em className="italic text-default-600">{children}</em>,
          code: ({ className, children }) => {
            const isBlock = className?.includes('language-');
            if (isBlock) {
              return (
                <code className={`${className} text-[11px] font-mono text-success leading-relaxed`}>
                  {children}
                </code>
              );
            }
            return (
              <code className="px-1.5 py-0.5 rounded-md bg-default-100 text-foreground text-[11px] font-mono border border-default-200">
                {children}
              </code>
            );
          },
          pre: ({ children }) => (
            <pre className="my-2.5 p-3 rounded-lg bg-default-50 overflow-x-auto border border-default-200">
              {children}
            </pre>
          ),
          blockquote: ({ children }) => (
            <blockquote className="my-2 pl-3 pr-2 py-1.5 border-l-[3px] border-default-300 bg-default-50 rounded-r-lg text-default-600">
              {children}
            </blockquote>
          ),
          hr: () => <hr className="my-3 border-default-200" />,
          table: ({ children }) => (
            <div className="my-2.5 overflow-x-auto rounded-lg border border-default-200">
              <table className="w-full text-[11px] border-collapse">{children}</table>
            </div>
          ),
          thead: ({ children }) => <thead className="bg-default-100">{children}</thead>,
          th: ({ children }) => (
            <th className="border-b border-default-200 px-3 py-2 text-left font-medium text-default-600">
              {children}
            </th>
          ),
          td: ({ children }) => (
            <td className="border-b border-default-100 px-3 py-2 text-default-700">{children}</td>
          ),
          tr: ({ children }) => (
            <tr className="hover:bg-default-50 transition-colors">{children}</tr>
          ),
          a: ({ children, href }) => {
            const isApi = href?.startsWith('/api/');
            const apiOrigin =
              window.location.port === '3001'
                ? `${window.location.protocol}//${window.location.hostname}:8081`
                : window.location.origin;
            const fullUrl = isApi ? `${apiOrigin}${href}` : (href ?? '');
            return (
              <a
                href={fullUrl}
                download
                target="_blank"
                rel="noreferrer"
                className="text-primary underline font-medium hover:opacity-80 transition-opacity inline-flex items-center gap-1"
              >
                {children}
              </a>
            );
          },
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
    images: m.images ?? undefined,
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
  onToolStart?: (tool: { name: string; args: unknown }) => void;
  onToolResult?: (tool: { name: string; result: unknown; result_preview?: string }) => void;
  onApprovalRequired?: (action: CopilotAction) => void;
  onDone: (data: StreamDone) => void;
  onError: (error: string) => void;
}

/**
 * 调用 SSE 流式端点，逐事件回调。
 *
 * SSE 事件格式：`event: <name>\ndata: <json>\n\n`
 * 事件类型：user_message / context / delta / tool_start / tool_result / approval_required / done / error
 */
export async function streamChat(
  url: string,
  token: string,
  query: string,
  callbacks: StreamCallbacks,
  modelId?: number,
  signal?: AbortSignal,
  images?: string[],
): Promise<void> {
  const response = await fetch(url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ query, model_id: modelId, images }),
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
          const data: unknown = JSON.parse(eventData);
          if (!data || typeof data !== 'object') continue;
          const payload = data as Record<string, unknown>;
          switch (eventType) {
            case 'user_message':
              callbacks.onUserMessage(data as CopilotMessageDTO);
              break;
            case 'context':
              callbacks.onContext(data as StreamContext);
              break;
            case 'delta':
              callbacks.onDelta(typeof payload.text === 'string' ? payload.text : '');
              break;
            case 'tool_start':
              callbacks.onToolStart?.(data as { name: string; args: unknown });
              break;
            case 'tool_result':
              callbacks.onToolResult?.(
                data as { name: string; result: unknown; result_preview?: string },
              );
              break;
            case 'approval_required':
              callbacks.onApprovalRequired?.(data as CopilotAction);
              break;
            case 'done':
              callbacks.onDone(data as StreamDone);
              streamFinished = true;
              break;
            case 'error':
              callbacks.onError(typeof payload.error === 'string' ? payload.error : '未知错误');
              streamFinished = true;
              break;
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
      <span
        className="w-1.5 h-1.5 rounded-full bg-primary animate-bounce"
        style={{ animationDelay: '0ms' }}
      />
      <span
        className="w-1.5 h-1.5 rounded-full bg-primary animate-bounce"
        style={{ animationDelay: '150ms' }}
      />
      <span
        className="w-1.5 h-1.5 rounded-full bg-primary animate-bounce"
        style={{ animationDelay: '300ms' }}
      />
      <span className="ml-1.5 text-[11px] text-default-400">{text}</span>
    </div>
  );
}

// ============ 报告导出 ============

/** 把当前会话消息列表构建为 Markdown 报告字符串 */
export function buildExportMarkdown(messages: MessageItem[]): string {
  let md = `# 智能诊断分析报告\n导出时间：${new Date().toLocaleString('zh-CN')}\n\n-------------------------------------------\n\n`;
  messages.forEach((m) => {
    const role = m.sender === 'user' ? '操作员' : '智能助手';
    md += `### [${m.timestamp}] ${role}\n\n`;
    if (m.llmStatus) md += `> 模型状态：${m.llmStatus}\n\n`;
    md += `${m.text}\n\n`;
    if (m.rootCause) md += `> **根因分析：**\n> ${m.rootCause}\n\n`;
    if (m.suggestedAction) md += `> **建议动作：**\n> ${m.suggestedAction}\n\n`;
    if (m.ladderAscii) md += `**信令交互梯形图：**\n\`\`\`text\n${m.ladderAscii}\n\`\`\`\n\n`;
    md += `-------------------------------------------\n\n`;
  });
  return md;
}

/** 顶部展示与一键下拉切换当前启用的 LLM 模型 */
export function ActiveModelBadge({
  activeModel,
  onModelChange,
  canManage = false,
}: {
  activeModel: { id?: number; provider: string; model: string } | null;
  onModelChange?: () => void;
  canManage?: boolean;
}) {
  const [configs, setConfigs] = useState<LlmConfigRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [switching, setSwitching] = useState(false);

  const fetchConfigs = async () => {
    setLoading(true);
    try {
      const list = await api.get<LlmConfigRecord[]>('/llm-configs');
      setConfigs(list);
    } catch {
      // 忽略
    } finally {
      setLoading(false);
    }
  };

  const handleSelect = async (idKey: React.Key) => {
    const id = Number(idKey);
    if (!id || id === activeModel?.id) return;
    setSwitching(true);
    try {
      await api.post(`/llm-configs/${id}/activate`);
      message.success('已切换大模型引擎');
      onModelChange?.();
    } catch (e) {
      message.error(e instanceof Error ? e.message : '切换失败');
    } finally {
      setSwitching(false);
    }
  };

  const activePreset = activeModel ? findPreset(activeModel.provider) : null;
  const activeLabel = activeModel
    ? `${activePreset?.label || activeModel.provider} · ${activeModel.model}`
    : '未配置大模型';

  if (!canManage) {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full border border-primary/20 bg-primary/10 px-3 py-1 text-xs font-medium text-primary">
        <Sparkles className="h-3.5 w-3.5" />
        {activeLabel}
      </span>
    );
  }

  return (
    <Dropdown
      placement="bottom-end"
      backdrop="transparent"
      onOpenChange={(open) => open && void fetchConfigs()}
    >
      <DropdownTrigger>
        <button
          type="button"
          className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-primary/10 border border-primary/20 hover:bg-primary/20 text-primary transition-all text-xs font-medium cursor-pointer select-none"
        >
          <Sparkles className="w-3.5 h-3.5 text-primary" />
          <span>{switching ? '正在切换...' : activeLabel}</span>
          <ChevronDown className="w-3.5 h-3.5 opacity-60 ml-0.5" />
        </button>
      </DropdownTrigger>
      <DropdownMenu
        aria-label="选择大模型引擎"
        variant="flat"
        className="w-72 p-1.5"
        onAction={handleSelect}
        emptyContent={
          <div className="p-3 text-center text-xs text-default-400">
            {loading ? '加载可用模型中...' : '暂无可用 LLM 配置'}
          </div>
        }
      >
        {configs.map((cfg) => {
          const providerLabel = findPreset(cfg.provider)?.label || cfg.provider;
          const isCurrent = cfg.id === activeModel?.id || (activeModel == null && cfg.is_active);
          return (
            <DropdownItem
              key={cfg.id}
              textValue={`${providerLabel} - ${cfg.model}`}
              className={`rounded-xl py-2 my-0.5 transition-colors ${isCurrent ? 'bg-primary/10 text-primary font-semibold' : ''}`}
            >
              <div className="flex items-center justify-between gap-2 w-full">
                <div className="flex items-center gap-2.5 min-w-0">
                  <div
                    className={`p-1.5 rounded-lg ${isCurrent ? 'bg-primary/20 text-primary' : 'bg-default-100 text-default-500'}`}
                  >
                    <Cpu className="w-3.5 h-3.5" />
                  </div>
                  <div className="flex flex-col min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="text-xs font-medium text-foreground truncate">
                        {providerLabel}
                      </span>
                      <span className="text-[10px] px-1.5 py-0.5 rounded bg-content3 font-mono text-default-500">
                        {cfg.model}
                      </span>
                    </div>
                    {cfg.name && (
                      <span className="text-[10px] text-default-400 truncate">{cfg.name}</span>
                    )}
                  </div>
                </div>
                {isCurrent && <Check className="w-4 h-4 text-primary shrink-0" />}
              </div>
            </DropdownItem>
          );
        })}
      </DropdownMenu>
    </Dropdown>
  );
}
