// IVR 节点类型枚举 - 支持多种操作行为
export type IvrNodeType =
  | 'start'            // 呼入入口 (DID 匹配)
  | 'prompt'           // 播放语音文件
  | 'tts'              // 文字转语音
  | 'collect_dtmf'     // 收集 DTMF 按键
  | 'menu'             // 多级菜单分支 (按 DTMF 路由到下级)
  | 'condition'        // 条件判断 (变量比较分支)
  | 'route'            // 智能路由 (按主叫/被叫/时间选路)
  | 'transfer_queue'   // 转接坐席队列
  | 'transfer_ext'     // 转接分机
  | 'transfer_pstn'    // 转接外线 (PSTN 中继)
  | 'voicemail'        // 留言录音
  | 'record'           // 录音节点
  | 'http_webhook'     // HTTP Webhook 调用
  | 'set_var'          // 设置上下文变量
  | 'asr'              // 语音识别
  | 'ai_agent'         // AI 智能体对话
  | 'loop'             // 循环跳转
  | 'hangup';          // 挂断

// 节点端口定义 (用于连线)
export interface NodePort {
  id: string;          // 端口唯一 id
  label: string;       // 端口标签 (如 "按1" / "匹配" / "默认")
  type: 'out' | 'in';  // 输入/输出端口
}

// IVR 节点
export interface IvrNode {
  id: string;
  type: IvrNodeType;
  title: string;
  description?: string;
  position: { x: number; y: number };
  config: Record<string, unknown>;
}

// IVR 连线
export interface IvrEdge {
  id: string;
  source: string;        // 源节点 id
  target: string;        // 目标节点 id
  sourcePort?: string;   // 源端口 id (menu 节点按按键区分)
  label?: string;        // 边标签 (如 "按1" / "超时")
}

// 完整 IVR 流程
export interface IvrFlow {
  id: string;
  name: string;
  description?: string;
  did?: string;              // 绑定的 DID 号码
  welcome_prompt?: string;   // 入口欢迎语音
  timeout_secs: number;      // 全局超时
  enabled: boolean;
  nodes: IvrNode[];
  edges: IvrEdge[];
  created_at?: string;
  updated_at?: string;
}

// 节点目录定义
export interface NodeCatalogEntry {
  type: IvrNodeType;
  title: string;
  description: string;
  icon: string;             // lucide icon name
  color: string;            // tailwind 类名 (bg + text)
  category: 'flow' | 'media' | 'routing' | 'integration' | 'system';
  defaultConfig: Record<string, unknown>;
  defaultPorts: NodePort[];
}

// 节点目录 (18 种节点完整定义)
export const NODE_CATALOG: NodeCatalogEntry[] = [
  {
    type: 'start',
    title: '呼入入口',
    description: 'DID 号码匹配进入 IVR 流程',
    icon: 'Play',
    color: 'bg-emerald-500/15 text-emerald-600 border-emerald-500/30',
    category: 'flow',
    defaultConfig: { did: '', welcome_prompt: 'welcome.wav' },
    defaultPorts: [{ id: 'out', label: '进入', type: 'out' }],
  },
  {
    type: 'prompt',
    title: '播放语音',
    description: '播放本地音频文件 (支持 wav/mp3)',
    icon: 'Volume2',
    color: 'bg-blue-500/15 text-blue-600 border-blue-500/30',
    category: 'media',
    defaultConfig: { audio_file: 'prompt.wav', interruptible: true, loop: 1 },
    defaultPorts: [{ id: 'out', label: '播放完成', type: 'out' }],
  },
  {
    type: 'tts',
    title: '文字转语音',
    description: '实时合成语音播放 (TTS)',
    icon: 'MessageSquare',
    color: 'bg-cyan-500/15 text-cyan-600 border-cyan-500/30',
    category: 'media',
    defaultConfig: { text: '您好，欢迎使用我们的服务', voice: 'female-zh-CN', speed: 1.0 },
    defaultPorts: [{ id: 'out', label: '播放完成', type: 'out' }],
  },
  {
    type: 'collect_dtmf',
    title: '收号',
    description: '收集用户 DTMF 按键输入',
    icon: 'Hash',
    color: 'bg-violet-500/15 text-violet-600 border-violet-500/30',
    category: 'flow',
    defaultConfig: { max_digits: 4, timeout_secs: 5, terminator: '#' },
    defaultPorts: [
      { id: 'collected', label: '完成收号', type: 'out' },
      { id: 'timeout', label: '超时', type: 'out' },
    ],
  },
  {
    type: 'menu',
    title: '多级菜单',
    description: '按 DTMF 按键路由到下级节点',
    icon: 'GitBranch',
    color: 'bg-purple-500/15 text-purple-600 border-purple-500/30',
    category: 'flow',
    defaultConfig: {
      prompt: '请按键选择服务',
      options: [
        { key: '1', label: '选项 1' },
        { key: '2', label: '选项 2' },
        { key: '0', label: '人工服务' },
      ],
    },
    defaultPorts: [
      { id: 'key-1', label: '按 1', type: 'out' },
      { id: 'key-2', label: '按 2', type: 'out' },
      { id: 'key-0', label: '按 0', type: 'out' },
      { id: 'default', label: '其他按键', type: 'out' },
    ],
  },
  {
    type: 'condition',
    title: '条件判断',
    description: '基于变量比较的条件分支',
    icon: 'GitFork',
    color: 'bg-amber-500/15 text-amber-600 border-amber-500/30',
    category: 'flow',
    defaultConfig: {
      variable: '${caller.region}',
      operator: '==',
      value: '北京',
    },
    defaultPorts: [
      { id: 'match', label: '匹配', type: 'out' },
      { id: 'nomatch', label: '不匹配', type: 'out' },
    ],
  },
  {
    type: 'route',
    title: '智能路由',
    description: '按主叫/被叫/时间/费率智能选路',
    icon: 'Route',
    color: 'bg-rose-500/15 text-rose-600 border-rose-500/30',
    category: 'routing',
    defaultConfig: {
      strategy: 'lowest_cost',
      fallback: 'reject',
      time_window: { start: '09:00', end: '18:00' },
    },
    defaultPorts: [
      { id: 'matched', label: '匹配路由', type: 'out' },
      { id: 'fallback', label: '回退', type: 'out' },
    ],
  },
  {
    type: 'transfer_queue',
    title: '转接坐席队列',
    description: '分配给指定坐席队列接待',
    icon: 'Users',
    color: 'bg-orange-500/15 text-orange-600 border-orange-500/30',
    category: 'routing',
    defaultConfig: { queue_id: 'queue-support', priority: 5, skill: 'general', timeout_secs: 60 },
    defaultPorts: [
      { id: 'answered', label: '坐席接听', type: 'out' },
      { id: 'timeout', label: '排队超时', type: 'out' },
    ],
  },
  {
    type: 'transfer_ext',
    title: '转接分机',
    description: '转接到内部 SIP 分机',
    icon: 'PhoneCall',
    color: 'bg-teal-500/15 text-teal-600 border-teal-500/30',
    category: 'routing',
    defaultConfig: { extension: '1001', timeout_secs: 30 },
    defaultPorts: [
      { id: 'answered', label: '接听', type: 'out' },
      { id: 'noanswer', label: '无应答', type: 'out' },
    ],
  },
  {
    type: 'transfer_pstn',
    title: '转接外线',
    description: '通过 PSTN 中继转接外部号码',
    icon: 'PhoneForwarded',
    color: 'bg-pink-500/15 text-pink-600 border-pink-500/30',
    category: 'routing',
    defaultConfig: { trunk_id: 'gw-telecom', target_number: '13800138000', caller_id: 'auto' },
    defaultPorts: [
      { id: 'answered', label: '接听', type: 'out' },
      { id: 'failed', label: '失败', type: 'out' },
    ],
  },
  {
    type: 'voicemail',
    title: '留言录音',
    description: '录制用户语音留言并存储',
    icon: 'Voicemail',
    color: 'bg-indigo-500/15 text-indigo-600 border-indigo-500/30',
    category: 'media',
    defaultConfig: { max_duration_secs: 60, prompt: '请在滴声后留言' },
    defaultPorts: [{ id: 'out', label: '完成', type: 'out' }],
  },
  {
    type: 'record',
    title: '录音节点',
    description: '对当前通话进行录音存储',
    icon: 'Mic',
    color: 'bg-fuchsia-500/15 text-fuchsia-600 border-fuchsia-500/30',
    category: 'media',
    defaultConfig: { format: 'wav', sample_rate: 8000, stereo: false },
    defaultPorts: [{ id: 'out', label: '继续', type: 'out' }],
  },
  {
    type: 'http_webhook',
    title: 'HTTP Webhook',
    description: '调用外部 HTTP 接口获取动态数据',
    icon: 'Webhook',
    color: 'bg-lime-500/15 text-lime-600 border-lime-500/30',
    category: 'integration',
    defaultConfig: {
      url: 'https://api.example.com/ivr/lookup',
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      timeout_secs: 5,
    },
    defaultPorts: [
      { id: 'success', label: '成功 (2xx)', type: 'out' },
      { id: 'error', label: '失败', type: 'out' },
    ],
  },
  {
    type: 'set_var',
    title: '设置变量',
    description: '写入上下文变量供后续节点使用',
    icon: 'Variable',
    color: 'bg-stone-500/15 text-stone-600 border-stone-500/30',
    category: 'flow',
    defaultConfig: { name: 'vip_level', value: 'gold' },
    defaultPorts: [{ id: 'out', label: '继续', type: 'out' }],
  },
  {
    type: 'asr',
    title: '语音识别',
    description: 'ASR 实时识别用户语音输入',
    icon: 'AudioLines',
    color: 'bg-sky-500/15 text-sky-600 border-sky-500/30',
    category: 'media',
    defaultConfig: { engine: 'whisper', language: 'zh-CN', max_duration_secs: 10 },
    defaultPorts: [
      { id: 'success', label: '识别成功', type: 'out' },
      { id: 'silence', label: '静音超时', type: 'out' },
    ],
  },
  {
    type: 'ai_agent',
    title: 'AI 智能体',
    description: '接入 LLM 实现全双工 AI 对话',
    icon: 'Sparkles',
    color: 'bg-gradient-to-r from-purple-500/15 to-pink-500/15 text-purple-600 border-purple-500/30',
    category: 'integration',
    defaultConfig: {
      agent_id: 'agent-gpt4o-realtime',
      system_prompt: '你是一名专业的客服代表',
      max_turns: 10,
      interruption: true,
    },
    defaultPorts: [
      { id: 'handover', label: '转人工', type: 'out' },
      { id: 'complete', label: '对话结束', type: 'out' },
    ],
  },
  {
    type: 'loop',
    title: '循环跳转',
    description: '循环回到指定节点 (带最大次数)',
    icon: 'Repeat',
    color: 'bg-yellow-500/15 text-yellow-600 border-yellow-500/30',
    category: 'flow',
    defaultConfig: { target_node_id: '', max_iterations: 3 },
    defaultPorts: [
      { id: 'loop', label: '继续循环', type: 'out' },
      { id: 'exit', label: '退出循环', type: 'out' },
    ],
  },
  {
    type: 'hangup',
    title: '挂断',
    description: '结束当前通话',
    icon: 'PhoneOff',
    color: 'bg-red-500/15 text-red-600 border-red-500/30',
    category: 'system',
    defaultConfig: { reason: 'normal', playbye: true },
    defaultPorts: [],
  },
];

export const NODE_CATALOG_MAP: Record<IvrNodeType, NodeCatalogEntry> = NODE_CATALOG.reduce(
  (acc, entry) => {
    acc[entry.type] = entry;
    return acc;
  },
  {} as Record<IvrNodeType, NodeCatalogEntry>
);

// 生成节点 id
export const genNodeId = (type: IvrNodeType): string =>
  `${type}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;

// 生成边 id
export const genEdgeId = (): string =>
  `edge-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
