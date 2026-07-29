//! Copilot 消息渲染组件
//!
//! 拆分自 copilot.tsx，包含：
//! - WelcomePanel：无消息时的欢迎页 + 预设查询按钮
//! - MessageBubble：单条消息渲染（头像/气泡/图片/CSV附件/根因/建议卡片）
//!
//! 纯展示组件，不含业务状态，所有数据通过 props 传入。

import {
  AlertTriangle,
  Bot,
  Check,
  Download,
  FileText,
  Lightbulb,
  Loader2,
  ShieldAlert,
  User,
  Wrench,
  X,
} from 'lucide-react';
import { Button, Chip, Spinner } from '@heroui/react';
import {
  DegradedBanner,
  LlmStateChip,
  MarkdownReport,
  StreamingIndicator,
  parseLlmError,
  parseLlmState,
  TOOL_LABELS,
  type MessageItem,
  type ToolCallTrace,
} from './copilot-shared';

// ============ 预设查询（对齐后端真实工具执行能力）============
// 仅 WelcomePanel 使用，故放在本文件而非 shared。

export const PRESETS = [
  {
    title: '生成每日汇报',
    desc: '帮我生成今日每日汇报，包含当日总结、呼叫情况、问题原因分析和建议',
  },
  { title: '诊断失败通话', desc: '帮我分析最新的呼叫失败记录并绘制信令交互梯形图' },
  {
    title: '批量导入分机',
    desc: '帮我把这段文本整理并批量导入分机：小王分机 8001 密码 123456，小张分机 8002 密码 888888',
  },
  { title: '配置前缀路由', desc: '添加一条号段路由，将前缀 010 开头的呼叫全部路由到主网关' },
  { title: '新增中继节点', desc: '新建一个名称为北京中继的网关，目标地址为 192.168.1.100' },
  {
    title: '创建客服菜单',
    desc: '创建一个客服语音菜单，按键 1 转接分机 8001，按键 2 转接分机 8002',
  },
  { title: '查询账户余额', desc: '查询当前所有计费账户余额，并给指定账户充值 1000 元' },
  { title: '配置风控规则', desc: '针对主叫前缀 9527 创建一条限频风控规则，上限 30 次' },
  { title: '定位异常通话', desc: '查询当前正在进行的并发通话列表，并定位异常通道' },
];

const WELCOME_TEXT =
  '您好，我是智能运维助手。我可以协助分析信令、定位通话故障、校验选路冲突，也可以在您确认后完成开户、路由、语音菜单和计费账户等配置操作。\n\n点击下方快捷入口，或直接描述您的需求。';

// ============ 欢迎页 ============

export interface WelcomePanelProps {
  onPresetClick: (desc: string) => void;
}

/** 无消息时显示的欢迎页：图标 + 标题 + 说明 + 预设查询按钮 */
export function WelcomePanel({ onPresetClick }: WelcomePanelProps) {
  return (
    <div className="flex flex-col items-center justify-center py-12 w-full">
      <div className="w-14 h-14 rounded-xl bg-default-100 border border-default-200 flex items-center justify-center text-primary mb-5">
        <Bot className="w-7 h-7" />
      </div>
      <h1 className="text-lg font-semibold text-foreground text-center mb-5">
        有什么我能帮你的吗？
      </h1>
      <div className="max-w-2xl mx-auto mb-6">
        <MarkdownReport content={WELCOME_TEXT} />
      </div>
      <div className="flex flex-wrap gap-2.5 justify-center max-w-2xl mx-auto">
        {PRESETS.map((p, idx) => (
          <button
            key={idx}
            onClick={() => onPresetClick(p.desc)}
            className="px-4 py-2 min-h-[38px] text-xs rounded-full border border-default-200 hover:border-primary/50 hover:bg-primary/5 text-default-600 hover:text-primary transition-colors font-medium"
          >
            {p.title}
          </button>
        ))}
      </div>
    </div>
  );
}

// ============ 加载中指示器 ============

export function MessagesLoading() {
  return (
    <div className="flex items-center justify-center py-12">
      <Spinner size="lg" />
    </div>
  );
}

// ============ 工具调用过程卡片 ============

/** 工具调用过程可视化：展示"正在调用 XXX"和"XXX 已返回"的卡片 */
export function ToolCallCard({
  trace,
  processing,
  canExecute,
  onApprove,
  onReject,
}: {
  trace: ToolCallTrace;
  processing: boolean;
  canExecute: boolean;
  onApprove: (actionId: string) => void;
  onReject: (actionId: string) => void;
}) {
  const label = TOOL_LABELS[trace.name] || trace.name;
  const isPending = trace.status === 'pending';
  const action = trace.action;
  const needsApproval = action?.status === 'pending' && !processing;
  const actionFailed = action?.status === 'failed' || action?.status === 'rejected';
  const riskLabel = action?.risk_level === 'high_risk' ? '高风险操作' : '配置变更';
  const statusLabel = needsApproval
    ? '待确认'
    : action?.status === 'approved'
      ? '已批准并执行'
      : action?.status === 'rejected'
        ? '已拒绝'
        : action?.status === 'failed'
          ? '执行失败'
          : action?.status === 'executing' || processing
            ? '执行中'
            : isPending
              ? '调用中'
              : '已完成';
  return (
    <div
      className={`rounded-lg border px-3 py-2 text-[11px] transition-all ${
        needsApproval
          ? 'bg-warning/10 border-warning/30 text-foreground'
          : isPending || processing
            ? 'bg-primary/5 border-primary/20 text-primary'
            : 'bg-content2 border-default-200 text-default-600'
      }`}
    >
      <div className="flex items-center gap-2">
        {needsApproval ? (
          <ShieldAlert className="w-3.5 h-3.5 shrink-0 text-warning" />
        ) : (
          <Wrench
            className={`w-3 h-3 shrink-0 ${isPending ? 'text-primary' : 'text-default-400'}`}
          />
        )}
        <span className="font-medium">{label}</span>
        {isPending || processing ? (
          <Loader2 className="w-3 h-3 animate-spin text-primary/70 ml-1" />
        ) : actionFailed ? (
          <X className="w-3 h-3 text-danger ml-1" />
        ) : (
          <Check className="w-3 h-3 text-success ml-1" />
        )}
        <span
          className={`ml-auto font-mono text-[10px] ${needsApproval ? 'text-warning' : 'text-default-400'}`}
        >
          {statusLabel}
        </span>
      </div>
      {action && (
        <div className="mt-2 border-t border-default-200/70 pt-2">
          <div className="flex items-center justify-between gap-2">
            <Chip
              size="sm"
              variant="flat"
              color={action.risk_level === 'high_risk' ? 'danger' : 'warning'}
              className="h-5 text-[10px]"
            >
              {riskLabel}
            </Chip>
            <span className="text-[10px] text-default-400">操作执行后可能改变系统数据</span>
          </div>
          <details className="mt-2 text-default-500">
            <summary className="cursor-pointer select-none text-[10px] hover:text-foreground">
              查看操作参数
            </summary>
            <pre className="mt-1.5 max-h-32 overflow-auto whitespace-pre-wrap break-all rounded-md bg-content1 p-2 text-[10px] text-default-600">
              {JSON.stringify(action.tool_arguments, null, 2)}
            </pre>
          </details>
          {trace.resultPreview && action.status !== 'pending' && (
            <details className="mt-2 text-default-500">
              <summary className="cursor-pointer select-none text-[10px] hover:text-foreground">
                查看执行结果
              </summary>
              <pre className="mt-1.5 max-h-32 overflow-auto whitespace-pre-wrap break-all rounded-md bg-content1 p-2 text-[10px] text-default-600">
                {trace.resultPreview}
              </pre>
            </details>
          )}
          {needsApproval && canExecute && (
            <div className="mt-2 flex justify-end gap-2">
              <Button
                size="sm"
                variant="flat"
                color="danger"
                isDisabled={processing}
                onPress={() => onReject(action.id)}
                startContent={<X className="h-3 w-3" />}
              >
                拒绝
              </Button>
              <Button
                size="sm"
                color="primary"
                isLoading={processing}
                onPress={() => onApprove(action.id)}
                startContent={!processing && <Check className="h-3 w-3" />}
              >
                批准执行
              </Button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ============ 单条消息气泡 ============

export interface MessageBubbleProps {
  message: MessageItem;
  sending: boolean;
  onImageClick: (url: string) => void;
  onFileClick: (file: { name: string; content: string }) => void;
  onCopyText: (text: string) => void;
  onApproveAction: (actionId: string) => void;
  onRejectAction: (actionId: string) => void;
  isActionUpdating: (actionId: string) => boolean;
  canExecute: boolean;
}

/** 单条消息渲染：头像 + 气泡 + 图片/CSV附件 + 工具调用 + 根因分析 + 建议动作 */
export function MessageBubble({
  message: m,
  sending,
  onImageClick,
  onFileClick,
  onCopyText,
  onApproveAction,
  onRejectAction,
  isActionUpdating,
  canExecute,
}: MessageBubbleProps) {
  const llmState = parseLlmState(m.llmStatus, m.llmEnabled);
  const llmError = llmState === 'degraded' ? parseLlmError(m.llmStatus) : '';
  const isStreaming = m.sender === 'bot' && m.text === '' && sending;
  const hasToolCalls = m.sender === 'bot' && m.toolCalls && m.toolCalls.length > 0;

  return (
    <div
      className={`flex gap-3 w-full ${m.sender === 'user' ? 'ml-auto flex-row-reverse max-w-3xl' : 'max-w-5xl'}`}
    >
      {/* 头像 */}
      <div
        className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 ${
          m.sender === 'user'
            ? 'bg-primary text-primary-foreground'
            : 'bg-default-100 border border-default-200 text-default-500'
        }`}
      >
        {m.sender === 'user' ? <User className="w-4 h-4" /> : <Bot className="w-4 h-4" />}
      </div>
      <div
        className={`flex flex-col gap-2.5 flex-1 min-w-0 ${m.sender === 'user' ? 'items-end' : 'items-start'}`}
      >
        {/* 工具调用过程卡片（bot 消息气泡上方） */}
        {hasToolCalls && (
          <div className="flex flex-col gap-1.5 w-full max-w-md">
            {m.toolCalls!.map((t, idx) => (
              <ToolCallCard
                key={t.action?.id ?? `${t.name}-${idx}`}
                trace={t}
                processing={t.action ? isActionUpdating(t.action.id) : false}
                canExecute={canExecute}
                onApprove={onApproveAction}
                onReject={onRejectAction}
              />
            ))}
          </div>
        )}
        {/* 消息气泡 */}
        <div
          className={`p-3.5 rounded-xl text-[13px] leading-6 w-fit max-w-full ${
            m.sender === 'user'
              ? 'bg-primary/10 text-foreground border border-primary/15 rounded-tr-sm'
              : 'bg-content1 text-foreground rounded-tl-sm border border-default-200'
          }`}
        >
          <div className="flex items-center justify-between text-[10px] mb-1.5 gap-2 text-default-400">
            <span className="flex items-center gap-2">
              <span>{m.sender === 'user' ? '操作员' : '智能助手'}</span>
              {m.sender === 'bot' && <LlmStateChip state={llmState} status={m.llmStatus} />}
            </span>
            <span>{m.timestamp}</span>
          </div>
          {m.sender === 'user' ? (
            <div className="flex flex-col gap-2">
              {/* 图片附件微缩图 */}
              {m.images && m.images.length > 0 && (
                <div className="flex flex-wrap gap-2 max-w-full my-1">
                  {m.images.map((imgUrl, idx) => (
                    <img
                      key={idx}
                      src={imgUrl}
                      alt={`分析识别截图-${idx + 1}`}
                      className="max-h-48 max-w-sm rounded-lg border border-default-200 cursor-pointer hover:opacity-90 transition-opacity object-contain bg-content1"
                      onClick={() => onImageClick(imgUrl)}
                    />
                  ))}
                </div>
              )}

              {/* CSV / 文本数据文件附件卡片 */}
              {m.files && m.files.length > 0 && (
                <div className="flex flex-col gap-2 my-1">
                  {m.files.map((file, idx) => (
                    <div
                      key={idx}
                      className="flex items-center justify-between p-2.5 rounded-lg bg-content1 border border-default-200 text-xs hover:border-primary/40 transition-colors"
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <div className="w-8 h-8 rounded-lg bg-default-100 flex items-center justify-center shrink-0">
                          <FileText className="w-4 h-4 text-default-500" />
                        </div>
                        <div className="flex flex-col min-w-0">
                          <span className="font-medium truncate text-foreground">{file.name}</span>
                          <span className="text-[10px] text-default-400">
                            {file.sizeStr || '数据文件'}
                          </span>
                        </div>
                      </div>
                      {file.content && (
                        <button
                          type="button"
                          onClick={() =>
                            onFileClick({
                              name: file.name as string,
                              content: file.content as string,
                            })
                          }
                          className="px-2.5 py-1 rounded-lg bg-primary/10 hover:bg-primary/15 text-[11px] font-medium text-primary transition-colors shrink-0 flex items-center gap-1 cursor-pointer"
                        >
                          <FileText className="w-3 h-3" /> 预览 CSV 数据
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              )}

              {m.text && <p className="whitespace-pre-wrap font-medium text-xs">{m.text}</p>}
            </div>
          ) : isStreaming ? (
            <StreamingIndicator />
          ) : (
            <>
              <MarkdownReport content={m.text} />
              <div className="flex items-center justify-end gap-2 mt-2 pt-1 border-t border-default-100/50 text-[10px] text-default-400">
                <button
                  type="button"
                  onClick={() => onCopyText(m.text)}
                  className="hover:text-primary transition-colors flex items-center gap-1 cursor-pointer"
                >
                  <Download className="w-3 h-3" /> 复制报告
                </button>
              </div>
            </>
          )}
          {llmState === 'degraded' && <DegradedBanner error={llmError} />}
        </div>

        {/* 根因分析卡片（warning 主题）*/}
        {m.rootCause && (
          <div className="w-full p-3.5 bg-warning/5 border border-warning/20 rounded-xl text-xs flex flex-col gap-1.5">
            <div className="flex items-center gap-1.5 text-foreground font-semibold">
              <AlertTriangle className="w-4 h-4" />
              <span>根因分析</span>
            </div>
            <div className="text-foreground text-[11px] pl-5 leading-relaxed">
              <MarkdownReport content={m.rootCause} />
            </div>
          </div>
        )}

        {/* 建议动作卡片（primary 主题）*/}
        {m.suggestedAction && (
          <div className="w-full p-3.5 bg-default-50 border border-default-200 rounded-xl text-xs flex flex-col gap-1.5">
            <div className="flex items-center gap-1.5 text-foreground font-semibold">
              <Lightbulb className="w-4 h-4" />
              <span>建议动作</span>
            </div>
            <div className="text-foreground text-[11px] pl-5 leading-relaxed">
              <MarkdownReport content={m.suggestedAction} />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
