import { useState } from 'react';
import {
  Card, CardBody, Button, Input, Chip, ScrollShadow, Divider
} from '@heroui/react';
import { Sparkles, Send, Bot, User, Terminal, AlertTriangle, ShieldCheck, RefreshCw, MessageSquare, Flame, HelpCircle, Activity, ChevronRight, Download } from 'lucide-react';
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

const PRESET_CARDS = [
  {
    title: '排查单通/挂断超时',
    desc: '排查 13800138000 为什么在 10:15 被挂断',
    icon: AlertTriangle,
    color: 'text-warning bg-warning/10 border-warning/20'
  },
  {
    title: '合成 SIP 交互梯形图',
    desc: '生成最新呼叫的完整 SIP Ladder Diagram 梯形图',
    icon: Terminal,
    color: 'text-primary bg-primary/10 border-primary/20'
  },
  {
    title: 'Deepfake 声纹防御审计',
    desc: '查询近期 AI 伪造声音拦截日志与硬中断记录',
    icon: ShieldCheck,
    color: 'text-success bg-success/10 border-success/20'
  },
  {
    title: '全网 QoS 健康检查',
    desc: '评估当前 CPS、RTP 丢包率与集群节点探活状态',
    icon: Activity,
    color: 'text-primary bg-primary/10 border-primary/20'
  }
];

export function CopilotPage() {
  const [messages, setMessages] = useState<MessageItem[]>([
    {
      id: 'msg-1',
      sender: 'bot',
      text: '您好！我是 vos-rs 电信级 LLM 智能运维 Copilot。我可以为您自动分析 SIP 抓包、动态绘制信令交互梯形图 (Call Ladder Diagram)、识别 QoS 异常并下发容灾切流策略。',
      timestamp: '11:30:00',
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
    <div className="flex flex-col gap-4 h-[calc(100vh-100px)]">
      {/* 顶部 AI Header（对齐 overview 的 Card 标题栏风格） */}
      <Card shadow="sm" className="shrink-0">
        <CardBody className="p-4 flex flex-wrap items-center justify-between gap-4 border-b border-divider">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-xl bg-primary/10 border border-primary/20 flex items-center justify-center text-primary">
              <Bot className="w-6 h-6" />
            </div>
            <div>
              <div className="flex items-center gap-2 mb-0.5">
                <h2 className="text-base font-bold text-foreground">LLM Telecom Copilot 智能运维与自愈</h2>
                <Chip size="sm" color="primary" variant="flat" startContent={<Sparkles className="w-3 h-3" />}>
                  AI-Native Autonomous
                </Chip>
              </div>
              <p className="text-tiny text-default-500">
                信令分析 · SIP 梯形图合成 · 根因定位 · 容灾热切流自愈
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="flat"
              onPress={handleExport}
              startContent={<Download className="w-4 h-4" />}
            >
              导出诊断报告
            </Button>
            <Button
              size="sm"
              variant="flat"
              color="primary"
              startContent={<RefreshCw className="w-4 h-4" />}
              onPress={() => setMessages([messages[0]])}
            >
              重置对话
            </Button>
          </div>
        </CardBody>
      </Card>

      {/* 主沉浸双栏布局 (Left Sidebar + Right Chat) */}
      <div className="flex-1 flex gap-4 min-h-0 overflow-hidden">
        {/* 左侧诊断快捷面板 */}
        <div className="w-80 flex flex-col gap-4 shrink-0 overflow-y-auto pr-1">
          <Card shadow="sm">
            <CardBody className="p-4 flex flex-col gap-3">
              <div className="flex items-center gap-2 text-small font-bold text-foreground pb-3 border-b border-divider">
                <Flame className="w-4 h-4 text-primary" />
                <span>AI 常用排障剧本 (Presets)</span>
              </div>

              <div className="flex flex-col gap-2.5">
                {PRESET_CARDS.map((card, idx) => {
                  const IconComp = card.icon;
                  return (
                    <div
                      key={idx}
                      onClick={() => handleSend(card.desc)}
                      className="p-3 rounded-xl border border-default-200 hover:border-primary/50 bg-content2/40 hover:bg-primary/5 cursor-pointer transition-all flex flex-col gap-1 group"
                    >
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                          <div className={`p-1.5 rounded-lg border ${card.color}`}>
                            <IconComp className="w-3.5 h-3.5" />
                          </div>
                          <span className="text-xs font-bold text-foreground group-hover:text-primary">
                            {card.title}
                          </span>
                        </div>
                        <ChevronRight className="w-3.5 h-3.5 text-default-400 group-hover:translate-x-0.5 transition-transform" />
                      </div>
                      <p className="text-[11px] text-default-500 line-clamp-2 pl-0.5">{card.desc}</p>
                    </div>
                  );
                })}
              </div>
            </CardBody>
          </Card>

          <div className="p-4 bg-primary/10 rounded-xl border border-primary/20 flex flex-col gap-2 text-tiny text-primary">
            <div className="flex items-center gap-1.5 font-bold">
              <HelpCircle className="w-4 h-4" />
              <span>智能提示词建议</span>
            </div>
            <p className="text-[11px] leading-relaxed text-default-500">
              您可以直接打字询问特定 Call-ID、主被叫号码、网关熔断记录或请求生成特定时间段的 SIP 交互梯形图。
            </p>
          </div>
        </div>

        {/* 右侧聊天沉浸主窗口 */}
        <Card shadow="sm" className="flex-1 flex flex-col overflow-hidden">
          <CardBody className="p-0 flex flex-col h-full overflow-hidden">
            {/* 消息滚动区域 */}
            <ScrollShadow className="flex-1 p-6 space-y-6 overflow-y-auto">
              {messages.map((m) => (
                <div
                  key={m.id}
                  className={`flex gap-4 max-w-4xl ${m.sender === 'user' ? 'ml-auto flex-row-reverse' : ''}`}
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

                    {/* 高亮渲染 SIP 信令梯形图 (Call Ladder Diagram ASCII) */}
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
            </ScrollShadow>

            <Divider />

            {/* 底部 Input 提问输入条 */}
            <div className="p-4 bg-content2/60 flex items-center gap-3">
              <Input
                variant="bordered"
                size="lg"
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
                    isLoading={loading}
                    onPress={() => handleSend()}
                    startContent={!loading && <Send className="w-3.5 h-3.5" />}
                  >
                    发送诊断
                  </Button>
                }
              />
            </div>
          </CardBody>
        </Card>
      </div>
    </div>
  );
}
