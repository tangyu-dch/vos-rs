//! Copilot 消息渲染组件
//!
//! 拆分自 copilot.tsx，包含：
//! - WelcomePanel：无消息时的欢迎页 + 预设查询按钮
//! - MessageBubble：单条消息渲染（头像/气泡/图片/CSV附件/根因/建议卡片）
//!
//! 纯展示组件，不含业务状态，所有数据通过 props 传入。

import {
  AlertTriangle, Bot, Download, FileText, Lightbulb, User,
} from 'lucide-react';
import { Spinner } from '@heroui/react';
import {
  DegradedBanner, LlmStateChip, MarkdownReport, StreamingIndicator,
  parseLlmError, parseLlmState, type MessageItem,
} from './copilot-shared';

// ============ 预设查询（对齐后端真实工具执行能力）============
// 仅 WelcomePanel 使用，故放在本文件而非 shared。

export const PRESETS = [
  { title: '🔍 诊断最近通话失败', desc: '帮我分析最新的呼叫失败记录并绘制 SIP 信令交互梯形图' },
  { title: '🪄 杂乱文本智能开户导入', desc: '帮我把这段文本整理并批量导入分机：小王分机 8001 密码 123456，小张分机 8002 密码 888888' },
  { title: '🚦 前缀路由选路配置', desc: '添加一条号段路由，将前缀 010 开头的呼叫全部路由到网关 gw_main' },
  { title: '🌐 新增中继网关节点', desc: '新建一个名称为北京中继 (gw_beijing) 的网关，目标 IP 192.168.1.100' },
  { title: '🌳 客服 IVR 菜单转接', desc: '创建一个客服 IVR 菜单，按键 1 转接分机 8001，按键 2 转接分机 8002' },
  { title: '💰 计费账户余额充值', desc: '查询当前所有计费账户余额，并给账户 acc_01 充值 1000 元' },
  { title: '🛡️ 拦截风控规则配置', desc: '针对主叫前缀 9527 创建一条限频风控规则，上限 30 次' },
  { title: '📞 实时并发通话拆线', desc: '查询当前正在进行的并发通话列表，并定位异常通道' },
];

const WELCOME_TEXT = '您好！我是 vos-rs 电信级 LLM 智能 Copilot。我拥有**全量软交换系统操控、选路冲突校验与智能排障能力**。您可以让我：分析 SIP 抓包并绘制梯形图、自动开户分机、配置前缀路由、创建 IVR 流程树、充值计费账户及挂断异常通道。\n\n点击下方快捷预设，或直接在输入框描述您的需求。';

// ============ 欢迎页 ============

export interface WelcomePanelProps {
  onPresetClick: (desc: string) => void;
}

/** 无消息时显示的欢迎页：图标 + 标题 + 说明 + 预设查询按钮 */
export function WelcomePanel({ onPresetClick }: WelcomePanelProps) {
  return (
    <div className="flex flex-col items-center justify-center py-12 w-full">
      <div className="w-20 h-20 rounded-3xl bg-gradient-to-br from-primary/20 to-primary/5 border border-primary/30 flex items-center justify-center text-primary mb-6 shadow-lg shadow-primary/10">
        <Bot className="w-10 h-10" />
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
            onClick={() => onPresetClick(p.desc)}
            className="px-4 py-2 min-h-[40px] text-xs rounded-full border border-default-200 hover:border-primary hover:bg-primary/10 text-default-600 hover:text-primary transition-all duration-200 shadow-sm font-medium"
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

// ============ 单条消息气泡 ============

export interface MessageBubbleProps {
  message: MessageItem;
  sending: boolean;
  onImageClick: (url: string) => void;
  onFileClick: (file: { name: string; content: string }) => void;
  onCopyText: (text: string) => void;
}

/** 单条消息渲染：头像 + 气泡 + 图片/CSV附件 + 根因分析 + 建议动作 */
export function MessageBubble({
  message: m, sending, onImageClick, onFileClick, onCopyText,
}: MessageBubbleProps) {
  const llmState = parseLlmState(m.llmStatus, m.llmEnabled);
  const llmError = llmState === 'degraded' ? parseLlmError(m.llmStatus) : '';
  const isStreaming = m.sender === 'bot' && m.text === '' && sending;

  return (
    <div
      className={`flex gap-4 w-full ${m.sender === 'user' ? 'ml-auto flex-row-reverse max-w-3xl' : 'max-w-full'}`}
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
      <div className={`flex flex-col gap-2.5 flex-1 min-w-0 ${m.sender === 'user' ? 'items-end' : 'items-start'}`}>
        {/* 消息气泡 */}
        <div
          className={`p-4 rounded-2xl text-xs leading-relaxed shadow-sm w-fit max-w-full ${
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
            <div className="flex flex-col gap-2">
              {/* 图片附件微缩图 */}
              {m.images && m.images.length > 0 && (
                <div className="flex flex-wrap gap-2 max-w-full my-1">
                  {m.images.map((imgUrl, idx) => (
                    <img
                      key={idx}
                      src={imgUrl}
                      alt={`分析识别截图-${idx + 1}`}
                      className="max-h-48 max-w-sm rounded-xl border border-primary-foreground/30 shadow-md cursor-pointer hover:opacity-90 hover:scale-[1.02] transition-all object-contain bg-black/20"
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
                      className="flex items-center justify-between p-2.5 rounded-xl bg-black/20 border border-primary-foreground/20 text-xs shadow-sm hover:border-primary-foreground/40 transition-colors"
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <div className="w-8 h-8 rounded-lg bg-primary-foreground/10 flex items-center justify-center shrink-0">
                          <FileText className="w-4 h-4 text-primary-foreground" />
                        </div>
                        <div className="flex flex-col min-w-0">
                          <span className="font-bold truncate text-primary-foreground">{file.name}</span>
                          <span className="text-[10px] text-primary-foreground/70">{file.sizeStr || '数据文件'}</span>
                        </div>
                      </div>
                      {file.content && (
                        <button
                          type="button"
                          onClick={() => onFileClick({ name: file.name as string, content: file.content as string })}
                          className="px-2.5 py-1 rounded-lg bg-primary-foreground/20 hover:bg-primary-foreground/30 text-[11px] font-medium text-primary-foreground transition-colors shrink-0 flex items-center gap-1 cursor-pointer"
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
          <div className="w-full p-3.5 bg-warning/10 border border-warning/30 rounded-xl text-xs flex flex-col gap-1.5 shadow-sm">
            <div className="flex items-center gap-1.5 text-warning font-bold">
              <AlertTriangle className="w-4 h-4" />
              <span>根因分析 (Root Cause)</span>
            </div>
            <div className="text-foreground text-[11px] pl-5 leading-relaxed">
              <MarkdownReport content={m.rootCause} />
            </div>
          </div>
        )}

        {/* 建议动作卡片（primary 主题）*/}
        {m.suggestedAction && (
          <div className="w-full p-3.5 bg-primary/10 border border-primary/30 rounded-xl text-xs flex flex-col gap-1.5 shadow-sm">
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
}
