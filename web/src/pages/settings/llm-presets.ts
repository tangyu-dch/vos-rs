// LLM 厂商预设：内置国内主流大模型厂商，同时支持自定义。
// 用户选择预设后自动填充 base_url 和推荐模型，API Key 需用户自行填写。

export interface LlmProviderPreset {
  provider: string;
  label: string;
  baseUrl: string;
  models: string[];
  // 厂商官网申请 API Key 的链接
  apiKeyUrl?: string;
}

/** 内置国内大厂 + OpenAI 预设 */
export const LLM_PROVIDER_PRESETS: LlmProviderPreset[] = [
  {
    provider: 'zhipu',
    label: '智谱 GLM',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    models: ['glm-4.7-flash', 'glm-4-flash', 'glm-4-plus', 'glm-4-long', 'glm-4-air'],
    apiKeyUrl: 'https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys',
  },
  {
    provider: 'qwen',
    label: '通义千问 (阿里 DashScope)',
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    models: ['qwen-turbo', 'qwen-plus', 'qwen-max', 'qwen-long', 'qwen2.5-72b-instruct'],
    apiKeyUrl: 'https://dashscope.console.aliyun.com/apiKey',
  },
  {
    provider: 'deepseek',
    label: 'DeepSeek 深度求索',
    baseUrl: 'https://api.deepseek.com/v1',
    models: ['deepseek-chat', 'deepseek-reasoner', 'deepseek-coder'],
    apiKeyUrl: 'https://platform.deepseek.com/api_keys',
  },
  {
    provider: 'wenxin',
    label: '百度文心一言 (千帆)',
    baseUrl: 'https://qianfan.baidubce.com/v2',
    models: ['ernie-4.0-8k-latest', 'ernie-4.0-turbo-8k', 'ernie-speed-128k', 'ernie-lite-8k'],
    apiKeyUrl: 'https://console.bce.baidu.com/qianfan/ais/console/applicationConsole/application',
  },
  {
    provider: 'moonshot',
    label: 'Kimi (月之暗面 Moonshot)',
    baseUrl: 'https://api.moonshot.cn/v1',
    models: ['moonshot-v1-8k', 'moonshot-v1-32k', 'moonshot-v1-128k'],
    apiKeyUrl: 'https://platform.moonshot.cn/console/api-keys',
  },
  {
    provider: 'openai',
    label: 'OpenAI (需代理)',
    baseUrl: 'https://api.openai.com/v1',
    models: ['gpt-4o', 'gpt-4o-mini', 'gpt-4-turbo', 'gpt-3.5-turbo'],
    apiKeyUrl: 'https://platform.openai.com/api-keys',
  },
  {
    provider: 'custom',
    label: '自定义 (OpenAI 兼容)',
    baseUrl: '',
    models: [],
  },
];

/** 根据 provider 标识查找预设 */
export function findPreset(provider: string): LlmProviderPreset | undefined {
  return LLM_PROVIDER_PRESETS.find((p) => p.provider === provider);
}

/** 对齐后端 LlmConfigRecord 结构 */
export interface LlmConfigRecord {
  id: number;
  name: string;
  provider: string;
  api_key: string;
  base_url: string;
  model: string;
  temperature: number;
  is_active: boolean;
  supports_vision?: boolean;
  supports_stt?: boolean;
  supports_tts?: boolean;
  created_at: string;
  updated_at: string;
}

/** 对齐后端 UpsertLlmConfigInput */
export interface UpsertLlmConfigInput {
  name: string;
  provider: string;
  api_key: string;
  base_url: string;
  model: string;
  temperature: number;
  supports_vision?: boolean;
  supports_stt?: boolean;
  supports_tts?: boolean;
}

/** API Key 脱敏：仅显示前 6 位和后 4 位 */
export function maskApiKey(key: string): string {
  if (!key) return '';
  if (key.length <= 12) return '••••••••';
  return `${key.slice(0, 6)}••••••••${key.slice(-4)}`;
}
