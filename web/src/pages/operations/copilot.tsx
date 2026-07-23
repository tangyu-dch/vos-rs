import { useState } from 'react';
import { Button, Input, Chip, ScrollShadow } from '@heroui/react';
import { Send, Bot, User, Terminal, AlertTriangle, ShieldCheck, RefreshCw, MessageSquare, Download } from 'lucide-react';
import { api } from '@/services/client';
import { message } from '@/utils/toast';

interface MessageItem {
  id: string;
  sender: 'user' | 'bot';
  text: string;
  rootCause?: string;
  suggestedAction?: string;
  ladderAscii?: string;
  timestamp: string;
}

const PRESETS = [
  { title: '排查 13800138000 挂断原因', desc: '排查 13800138000 为什么在 10:15 被挂断' },
  { title: '合成最新呼叫 SIP 梯形图', desc: '生成最新呼叫的完整 SIP Ladder Diagram 梯形图' },
  { title: '查看 AI 伪造声音拦截日志', desc: '查询近期 AI 伪造声音拦截日志与硬中断记录' },
  { title: '评估当前 CPS 与丢包率', desc: '评估当前 CPS、RTP 丢包率与集群节点探活状态' },
  { title: '推荐高可用网关路由规则', desc: '推荐符合电信级高可用的路由网关策略' },
  { title: '分析 DID 号码的日常使用成本', desc: '分析当前系统中 DID 号码的日常租用与呼出成本' },
  { title: '检查计费余额预警限额', desc: '查询计费账户余额不足防欠费的熔断配置' },
  { title: '导出近期 SIP 注册异常审计', desc: '审计并分析近期分机与外部中继注册失败的原因' },
];

export function CopilotPage() {
  const [messages, setMessages] = useState<MessageItem[]>([
    {
      id: 'msg-1',
      sender: 'bot',
      text: '您好！我是 vos-rs 电信级 LLM 智能运维 Copilot。我可以为您自动分析 SIP 抓包、动态绘制信令交互梯形图 (Call Ladder Diagram)、识别 QoS 异常并下发容灾切流策略。',
      timestamp: new Date().toLocaleTimeString(),
    }
  ]);
  const [inputQuery, setInputQuery] = useState('');
  const [loading, setLoading] = useState(false);

  const handleSend = async (queryText?: string) => {
    const query = queryText || inputQuery;
    if (!query.trim() || loading) return;

    const userMsg: MessageItem = {
      id: `user-${Date.now()}`,
      sender: 'user',
      text: query,
      timestamp: new Date().toLocaleTimeString(),
    };

    setMessages((prev) => [...prev, userMsg]);
    if (!queryText) setInputQuery('');
    setLoading(true);

    try {
      const res: any = await api.post('/copilot/chat', { query });
      const botMsg: MessageItem = {
        id: `bot-${Date.now()}`,
        sender: 'bot',
        text: res.analysis_report || '分析完成。',
        rootCause: res.root_cause,
        suggestedAction: res.suggested_action,
        ladderAscii: res.ladder_diagram_ascii,
        timestamp: new Date().toLocaleTimeString(),
      };
      setMessages((prev) => [...prev, botMsg]);
    } catch {
      const errorMsg: MessageItem = {
        id: `bot-err-${Date.now()}`,
        sender: 'bot',
        text: '调取 LLM Telecom Copilot 诊断分析失败，请检查后端 API 服务连接。',
        timestamp: new Date().toLocaleTimeString(),
      };
      setMessages((prev) => [...prev, errorMsg]);
    } finally {
      setLoading(false);
    }
  };

  const handleExport = () => {
    if (messages.length <= 1) {
      message.warning('没有可导出的诊断记录');
      return;
    }

    let mdContent = `# LLM Telecom Copilot 诊断分析报告\n`;
    mdContent += `导出时间: ${new Date().toLocaleString()}\n\n`;
    mdContent += `-------------------------------------------\n\n`;

    messages.forEach((m) => {
      const role = m.sender === 'user' ? 'OPERATOR (操作员)' : 'TELECOM COPILOT (智能运维助手)';
      mdContent += `### [${m.timestamp}] ${role}\n\n`;
      mdContent += `${m.text}\n\n`;

      if (m.rootCause) {
        mdContent += `> **故障根因定位 (Root Cause):**\n`;
        mdContent += `> ${m.rootCause}\n\n`;
      }

      if (m.suggestedAction) {
        mdContent += `> **自动自愈策略 (Self-Healing Action):**\n`;
        mdContent += `> ${m.suggestedAction}\n\n`;
      }

      if (m.ladderAscii) {
        mdContent += `**SIP 信令交互梯形图 (Call Ladder Diagram):**\n`;
        mdContent += `\`\`\`text\n`;
        mdContent += `${m.ladderAscii}\n`;
        mdContent += `\`\`\`\n\n`;
      }

      mdContent += `-------------------------------------------\n\n`;
    });

    const blob = new Blob(['\ufeff' + mdContent], { type: 'text/markdown;charset=utf-8;' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.setAttribute('href', url);
    link.setAttribute('download', `Copilot_Diagnosis_Report_${new Date().toISOString().slice(0, 10)}.md`);
    link.style.visibility = 'hidden';
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    message.success('已成功导出 Copilot 诊断分析报告 (Markdown 格式)');
  };

  return (
    <div className="h-[calc(100vh-100px)] flex flex-col relative bg-transparent">
      {/* 顶部悬浮操作按钮 */}
      {messages.length > 1 && (
        <div className="absolute top-0 right-4 z-10 flex gap-2">
          <Button
            size="sm"
            variant="flat"
            onPress={handleExport}
            startContent={<Download className="w-3.5 h-3.5" />}
          >
            导出报告
          </Button>
          <Button
            size="sm"
            variant="flat"
            color="primary"
            startContent={<RefreshCw className="w-3.5 h-3.5" />}
            onPress={() => setMessages([messages[0]])}
          >
            重置
          </Button>
        </div>
      )}

      {/* 主沉浸聊天区 */}
      <div className="flex-1 flex flex-col min-h-0 justify-between items-center w-full">
        <ScrollShadow className="w-full flex-1 px-4 py-6 space-y-6 overflow-y-auto min-h-0">
          <div className="max-w-3xl mx-auto w-full space-y-6">
            {/* 极简欢迎词与建议芯片 (仅在无聊天对话时展示) */}
            {messages.length === 1 && (
              <div className="flex flex-col items-center justify-center py-12 w-full">
                <div className="w-12 h-12 rounded-2xl bg-primary/10 border border-primary/20 flex items-center justify-center text-primary mb-6 animate-pulse">
                  <Bot className="w-7 h-7" />
                </div>
                <h1 className="text-xl font-bold text-slate-800 dark:text-slate-200 text-center mb-8">
                  有什么我能帮你的吗？
                </h1>
                
                <div className="flex flex-wrap gap-2.5 justify-center max-w-2xl mx-auto">
                  {PRESETS.map((p, idx) => (
                    <button
                      key={idx}
                      onClick={() => handleSend(p.desc)}
                      className="px-4 py-2 text-xs rounded-full border border-default-200 hover:border-primary/50 bg-content1 hover:bg-primary/5 text-default-600 hover:text-primary transition-all duration-200 shadow-sm font-medium"
                    >
                      {p.title}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {/* 对话消息展示 */}
            {messages.length > 1 && messages.map((m) => (
              <div
                key={m.id}
                className={`flex gap-4 max-w-3xl ${m.sender === 'user' ? 'ml-auto flex-row-reverse' : ''}`}
              >
                <div
                  className={`w-10 h-10 rounded-2xl flex items-center justify-center shrink-0 font-bold ${
                    m.sender === 'user'
                      ? 'bg-primary text-primary-foreground'
                      : 'bg-primary/10 border border-primary/20 text-primary'
                  }`}
                >
                  {m.sender === 'user' ? <User className="w-5 h-5" /> : <Bot className="w-5 h-5" />}
                </div>

                <div className="flex flex-col gap-2.5 max-w-2xl">
                  <div
                    className={`p-4 rounded-2xl text-xs leading-relaxed ${
                      m.sender === 'user'
                        ? 'bg-primary text-primary-foreground rounded-tr-none'
                        : 'bg-content2 text-foreground rounded-tl-none border border-default-200'
                    }`}
                  >
                    <div className="flex items-center justify-between text-[10px] opacity-70 mb-1.5 font-mono">
                      <span>{m.sender === 'user' ? 'OPERATOR' : 'TELECOM COPILOT AGENT'}</span>
                      <span>{m.timestamp}</span>
                    </div>
                    <p className="whitespace-pre-wrap font-medium text-xs">{m.text}</p>
                  </div>

                  {/* 故障根因分析 */}
                  {m.rootCause && (
                    <div className="p-3.5 bg-warning/10 border border-warning/20 rounded-xl text-xs flex flex-col gap-1">
                      <div className="flex items-center gap-1.5 text-warning font-bold">
                        <AlertTriangle className="w-4 h-4" />
                        <span>故障根因定位 (Root Cause):</span>
                      </div>
                      <span className="text-foreground font-mono text-[11px] pl-5">{m.rootCause}</span>
                    </div>
                  )}

                  {/* 自动自愈策略下发 */}
                  {m.suggestedAction && (
                    <div className="p-3.5 bg-success/10 border border-success/20 rounded-xl text-xs flex flex-col gap-1">
                      <div className="flex items-center gap-1.5 text-success font-bold">
                        <ShieldCheck className="w-4 h-4" />
                        <span>自动自愈策略执行 (Self-Healing Action):</span>
                      </div>
                      <span className="text-foreground font-mono text-[11px] pl-5">{m.suggestedAction}</span>
                    </div>
                  )}

                  {/* 高亮渲染 SIP 信令梯形图 */}
                  {m.ladderAscii && (
                    <div className="p-4 bg-content2 rounded-2xl border border-default-200 flex flex-col gap-2">
                      <div className="flex items-center justify-between text-[11px] text-primary font-mono font-bold pb-2 border-b border-divider">
                        <div className="flex items-center gap-1.5">
                          <Terminal className="w-3.5 h-3.5 text-primary" />
                          <span>SIP Call Ladder Diagram (信令交互梯形图)</span>
                        </div>
                        <Chip size="sm" color="primary" variant="flat" className="text-[10px]">
                          Generated
                        </Chip>
                      </div>
                      <pre className="text-[11px] font-mono text-success overflow-x-auto whitespace-pre leading-tight py-2">
                        {m.ladderAscii}
                      </pre>
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </ScrollShadow>

        {/* 底部浮动输入框胶囊 */}
        <div className="w-full px-4 py-4 shrink-0 bg-transparent">
          <div className="w-full max-w-3xl mx-auto rounded-3xl border border-default-300 dark:border-slate-800 bg-content1 shadow-lg p-2 flex items-center gap-2">
            <Input
              variant="flat"
              classNames={{
                inputWrapper: "bg-transparent shadow-none hover:bg-transparent focus-within:bg-transparent",
                input: "text-sm",
              }}
              placeholder="询问 Copilot 排查通话问题 (如: 查一下刚才 13800138000 为什么被断开...)"
              value={inputQuery}
              onValueChange={setInputQuery}
              onKeyDown={(e) => e.key === 'Enter' && handleSend()}
              isDisabled={loading}
              startContent={<MessageSquare className="w-4 h-4 text-default-400" />}
              endContent={
                <Button
                  size="sm"
                  color="primary"
                  className="rounded-2xl px-4 text-white font-bold"
                  isLoading={loading}
                  onPress={() => handleSend()}
                  startContent={!loading && <Send className="w-3.5 h-3.5" />}
                >
                  发送
                </Button>
              }
            />
          </div>
        </div>
      </div>
    </div>
  );
}
