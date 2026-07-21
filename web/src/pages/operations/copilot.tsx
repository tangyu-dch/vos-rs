import { useState } from 'react';
import {
  Card, CardBody, Button, Input, Chip, ScrollShadow, Divider
} from '@heroui/react';
import { Sparkles, Send, Bot, User, Terminal, AlertTriangle, ShieldCheck, RefreshCw, MessageSquare, Flame, HelpCircle, Activity, ChevronRight } from 'lucide-react';
import { api } from '@/services/client';

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
    color: 'text-amber-500 bg-amber-500/10 border-amber-500/20'
  },
  {
    title: '合成 SIP 交互梯形图',
    desc: '生成最新呼叫的完整 SIP Ladder Diagram 梯形图',
    icon: Terminal,
    color: 'text-purple-500 bg-purple-500/10 border-purple-500/20'
  },
  {
    title: 'Deepfake 声纹防御审计',
    desc: '查询近期 AI 伪造声音拦截日志与硬中断记录',
    icon: ShieldCheck,
    color: 'text-emerald-500 bg-emerald-500/10 border-emerald-500/20'
  },
  {
    title: '全网 QoS 健康检查',
    desc: '评估当前 CPS、RTP 丢包率与集群节点探活状态',
    icon: Activity,
    color: 'text-blue-500 bg-blue-500/10 border-blue-500/20'
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

  return (
    <div className="flex flex-col gap-5 h-[calc(100vh-100px)]">
      {/* 顶部现代化 AI Header */}
      <div className="flex items-center justify-between p-4 px-6 bg-gradient-to-r from-purple-900/30 via-slate-900 to-indigo-900/30 backdrop-blur-md rounded-2xl border border-purple-500/20 shadow-sm shrink-0">
        <div className="flex items-center gap-3.5">
          <div className="w-10 h-10 rounded-xl bg-purple-500/20 border border-purple-400/30 flex items-center justify-center text-purple-400 shadow-inner">
            <Bot className="w-6 h-6 animate-pulse" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-extrabold text-slate-800 dark:text-slate-100">LLM Telecom Copilot 智能运维与自愈</h2>
              <Chip size="sm" color="secondary" variant="flat" className="font-bold border border-purple-400/30">
                <Sparkles className="w-3 h-3 text-yellow-400 inline mr-1" />
                AI-Native Autonomous
              </Chip>
            </div>
            <p className="text-xs text-slate-500 dark:text-slate-400">
              信令分析 · SIP 梯形图合成 · 根因定位 · 容灾热切流自愈
            </p>
          </div>
        </div>

        <Button
          size="sm"
          variant="flat"
          color="secondary"
          className="font-bold"
          startContent={<RefreshCw className="w-3.5 h-3.5" />}
          onPress={() => setMessages([messages[0]])}
        >
          重置对话
        </Button>
      </div>

      {/* 主沉浸双栏布局 (Left Sidebar + Right Chat) */}
      <div className="flex-1 flex gap-5 min-h-0 overflow-hidden">
        {/* 左侧诊断快捷面板 */}
        <div className="w-80 flex flex-col gap-4 shrink-0 overflow-y-auto pr-1">
          <Card className="shadow-sm border border-slate-200/80 dark:border-slate-800">
            <CardBody className="p-4 flex flex-col gap-3">
              <div className="flex items-center gap-2 text-xs font-bold text-slate-800 dark:text-slate-200 pb-2 border-b border-slate-100 dark:border-slate-800">
                <Flame className="w-4 h-4 text-purple-500" />
                <span>AI 常用排障剧本 (Presets)</span>
              </div>

              <div className="flex flex-col gap-2.5">
                {PRESET_CARDS.map((card, idx) => {
                  const IconComp = card.icon;
                  return (
                    <div
                      key={idx}
                      onClick={() => handleSend(card.desc)}
                      className="p-3 rounded-xl border border-slate-200/60 dark:border-slate-800 hover:border-purple-500/50 bg-slate-50/50 dark:bg-slate-900/50 hover:bg-purple-500/5 cursor-pointer transition-all flex flex-col gap-1 group"
                    >
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                          <div className={`p-1.5 rounded-lg border ${card.color}`}>
                            <IconComp className="w-3.5 h-3.5" />
                          </div>
                          <span className="text-xs font-bold text-slate-800 dark:text-slate-200 group-hover:text-purple-600 dark:group-hover:text-purple-400">
                            {card.title}
                          </span>
                        </div>
                        <ChevronRight className="w-3.5 h-3.5 text-slate-400 group-hover:translate-x-0.5 transition-transform" />
                      </div>
                      <p className="text-[11px] text-slate-500 line-clamp-2 pl-0.5">{card.desc}</p>
                    </div>
                  );
                })}
              </div>
            </CardBody>
          </Card>

          <div className="p-4 bg-purple-500/10 rounded-2xl border border-purple-500/20 flex flex-col gap-2 text-xs text-purple-700 dark:text-purple-300">
            <div className="flex items-center gap-1.5 font-bold">
              <HelpCircle className="w-4 h-4" />
              <span>智能提示词建议</span>
            </div>
            <p className="text-[11px] leading-relaxed text-purple-600/90 dark:text-purple-300/90">
              您可以直接打字询问特定 Call-ID、主被叫号码、网关熔断记录或请求生成特定时间段的 SIP 交互梯形图。
            </p>
          </div>
        </div>

        {/* 右侧聊天沉浸主窗口 */}
        <Card className="flex-1 shadow-sm border border-slate-200/80 dark:border-slate-800 flex flex-col overflow-hidden">
          <CardBody className="p-0 flex flex-col h-full overflow-hidden">
            {/* 消息滚动区域 */}
            <ScrollShadow className="flex-1 p-6 space-y-6 overflow-y-auto">
              {messages.map((m) => (
                <div
                  key={m.id}
                  className={`flex gap-4 max-w-4xl ${m.sender === 'user' ? 'ml-auto flex-row-reverse' : ''}`}
                >
                  <div
                    className={`w-10 h-10 rounded-2xl flex items-center justify-center shrink-0 font-bold shadow-xs ${
                      m.sender === 'user'
                        ? 'bg-gradient-to-r from-purple-600 to-indigo-600 text-white'
                        : 'bg-purple-500/15 border border-purple-500/30 text-purple-600 dark:text-purple-400'
                    }`}
                  >
                    {m.sender === 'user' ? <User className="w-5 h-5" /> : <Bot className="w-5 h-5" />}
                  </div>

                  <div className="flex flex-col gap-2.5 max-w-2xl">
                    <div
                      className={`p-4 rounded-2xl text-xs leading-relaxed ${
                        m.sender === 'user'
                          ? 'bg-purple-600 text-white rounded-tr-none shadow-md'
                          : 'bg-white dark:bg-slate-900 text-slate-800 dark:text-slate-100 rounded-tl-none border border-slate-200/80 dark:border-slate-800 shadow-sm'
                      }`}
                    >
                      <div className="flex items-center justify-between text-[10px] opacity-70 mb-1.5 font-mono">
                        <span>{m.sender === 'user' ? 'OPERATOR' : 'TELECOM COPILOT AGENT'}</span>
                        <span>{m.timestamp}</span>
                      </div>
                      <p className="whitespace-pre-wrap font-medium text-xs text-slate-700 dark:text-slate-200">{m.text}</p>
                    </div>

                    {/* 故障根因分析 */}
                    {m.rootCause && (
                      <div className="p-3.5 bg-amber-500/10 border border-amber-500/20 rounded-xl text-xs flex flex-col gap-1">
                        <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400 font-bold">
                          <AlertTriangle className="w-4 h-4" />
                          <span>故障根因定位 (Root Cause):</span>
                        </div>
                        <span className="text-slate-700 dark:text-slate-300 font-mono text-[11px] pl-5">{m.rootCause}</span>
                      </div>
                    )}

                    {/* 自动自愈策略下发 */}
                    {m.suggestedAction && (
                      <div className="p-3.5 bg-emerald-500/10 border border-emerald-500/20 rounded-xl text-xs flex flex-col gap-1">
                        <div className="flex items-center gap-1.5 text-emerald-600 dark:text-emerald-400 font-bold">
                          <ShieldCheck className="w-4 h-4" />
                          <span>自动自愈策略执行 (Self-Healing Action):</span>
                        </div>
                        <span className="text-slate-700 dark:text-slate-300 font-mono text-[11px] pl-5">{m.suggestedAction}</span>
                      </div>
                    )}

                    {/* 高亮渲染 SIP 信令梯形图 (Call Ladder Diagram ASCII) */}
                    {m.ladderAscii && (
                      <div className="p-4 bg-slate-950 rounded-2xl border border-slate-800 flex flex-col gap-2 shadow-inner">
                        <div className="flex items-center justify-between text-[11px] text-purple-400 font-mono font-bold pb-2 border-b border-slate-800/80">
                          <div className="flex items-center gap-1.5">
                            <Terminal className="w-3.5 h-3.5 text-purple-400" />
                            <span>SIP Call Ladder Diagram (信令交互梯形图)</span>
                          </div>
                          <Chip size="sm" color="secondary" variant="flat" className="text-[10px]">
                            Generated
                          </Chip>
                        </div>
                        <pre className="text-[11px] font-mono text-emerald-400 overflow-x-auto whitespace-pre leading-tight py-2">
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
            <div className="p-4 bg-slate-50/80 dark:bg-slate-950/80 flex items-center gap-3">
              <Input
                variant="bordered"
                size="lg"
                placeholder="询问 Copilot 排查通话问题 (如: 查一下刚才 13800138000 为什么被断开...)"
                value={inputQuery}
                onValueChange={setInputQuery}
                onKeyDown={(e) => e.key === 'Enter' && handleSend()}
                isDisabled={loading}
                startContent={<MessageSquare className="w-4 h-4 text-slate-400" />}
                endContent={
                  <Button
                    size="sm"
                    color="secondary"
                    className="font-bold bg-purple-600 text-white shadow-md"
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
