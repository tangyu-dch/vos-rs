// 系统管理 - 核心运行参数设置
// 从 console.tsx 拆分

import { useCallback, useEffect, useState } from 'react';
import {
  Button, Card, CardBody, Chip, Input, Switch, Spinner,
} from '@heroui/react';
import { RefreshCw } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import type { Entity } from '@/services/resources';

type ConfigKind = 'text' | 'number' | 'decimal' | 'boolean' | 'secret';
interface ConfigField {
  key: string;
  label: string;
  kind?: ConfigKind;
  hint: string;
  fullWidth?: boolean;
}
interface ConfigGroup {
  key: string;
  label: string;
  description: string;
  fields: ConfigField[];
}

const systemConfigGroups: ConfigGroup[] = [
  {
    key: 'sip', label: 'SIP 与会话', description: '认证域与呼叫会话计时器。', fields: [
      { key: 'realm', label: '认证 Realm', hint: 'Digest 认证域；存在分机时不可修改' },
      { key: 'session_expires_gateway', label: '网关会话时长', kind: 'number', hint: '单位：秒' },
      { key: 'session_expires_caller', label: '主叫会话时长', kind: 'number', hint: '单位：秒' },
    ],
  },
  {
    key: 'routing', label: '路由与中继', description: '路由运行依赖的中继健康探测。', fields: [
      { key: 'gateway_health_checks_enabled', label: '中继健康检查', kind: 'boolean', hint: '定期探测中继可用状态' },
    ],
  },
  {
    key: 'media', label: '媒体', description: 'RTP 地址学习、防欺骗与质量指标。', fields: [
      { key: 'rtp_symmetric_learning', label: '对称 RTP 学习', kind: 'boolean', hint: '从首个有效媒体包学习源地址' },
      { key: 'rtp_anti_spoofing', label: 'RTP 防欺骗', kind: 'boolean', hint: '拒绝非预期媒体源' },
      { key: 'rtp_source_relearn_secs', label: '媒体源重新学习窗口', kind: 'number', hint: '单位：秒' },
      { key: 'media_metrics_log', label: '媒体指标日志', kind: 'boolean', hint: '输出媒体质量统计日志' },
    ],
  },
  {
    key: 'recording', label: '录音', description: '录音任务、存储容量与文件生命周期。', fields: [
      { key: 'recording_enabled', label: '启用录音', kind: 'boolean', hint: '允许系统创建通话录音' },
      { key: 'recording_dir', label: '录音目录', hint: '节点本地录音文件根目录', fullWidth: true },
      { key: 'recording_workers', label: '录音工作线程', kind: 'number', hint: '异步落盘工作线程数' },
      { key: 'recording_queue_capacity', label: '录音队列容量', hint: '等待写入的任务上限' },
      { key: 'recording_retention_secs', label: '录音保留时长', kind: 'number', hint: '单位：秒' },
      { key: 'recording_min_free_bytes', label: '最小磁盘余量', kind: 'number', hint: '单位：字节' },
      { key: 'recording_max_file_bytes', label: '单文件上限', kind: 'number', hint: '单位：字节' },
      { key: 'recording_max_duration_secs', label: '单次录音时长上限', kind: 'number', hint: '单位：秒' },
    ],
  },
  {
    key: 'llm_integration', label: '大模型与 AI Voice 配置', description: '配置 OpenAI / Gemini / DeepSeek / 本地 vLLM 热生效参数。', fields: [
      { key: 'llm_enabled', label: '启用 LLM 对接', kind: 'boolean', hint: '允许呼叫中心与 Copilot 对接大模型' },
      { key: 'llm_provider', label: '模型提供商 (Provider)', hint: 'openai | gemini | deepseek | local_vllm | ollama' },
      { key: 'llm_base_url', label: 'LLM Endpoint (Base URL)', hint: '例如 https://api.openai.com/v1 或 http://localhost:11434/v1', fullWidth: true },
      { key: 'llm_api_key', label: 'LLM API Key', kind: 'secret', hint: '用于访问大模型的授权 Key (如 sk-proj-xxx)', fullWidth: true },
      { key: 'llm_model', label: '默认模型名称 (Model)', hint: '例如 gpt-4o-realtime-preview 或 deepseek-chat' },
      { key: 'llm_temperature', label: '采样温度 (Temperature)', kind: 'decimal', hint: '0.0 ~ 1.0 创造力系数' },
    ],
  },
  {
    key: 'billing', label: '计费与 CDR', description: '余额风控、结算与话单持久化。', fields: [
      { key: 'balance_enforcement_enabled', label: '余额强制校验', kind: 'boolean', hint: '呼叫前校验账户可用余额' },
      { key: 'billing_settlement_enabled', label: '启用计费结算', kind: 'boolean', hint: '通话结束后执行费用结算' },
      { key: 'cdr_persistence_enabled', label: 'CDR 持久化', kind: 'boolean', hint: '写入通话详单存储' },
      { key: 'cdr_queue_capacity', label: 'CDR 队列容量', kind: 'number', hint: '等待持久化的话单上限' },
    ],
  },
  {
    key: 'security', label: 'SBC 与 TLS', description: '边界限流及 SIP TLS 连接安全。', fields: [
      { key: 'sbc_rate_limit_capacity', label: '令牌桶容量', kind: 'decimal', hint: '单一来源允许的突发请求量' },
      { key: 'sbc_rate_limit_fill_rate', label: '令牌补充速率', kind: 'decimal', hint: '每秒补充令牌数' },
      { key: 'sbc_max_concurrency', label: 'SBC 最大并发', kind: 'number', hint: '边界层并发会话上限' },
      { key: 'tls_bind_addr', label: 'TLS 监听地址', hint: '例如 0.0.0.0:5061' },
      { key: 'tls_cert_path', label: 'TLS 证书路径', hint: 'PEM 证书文件路径', fullWidth: true },
      { key: 'tls_key_path', label: 'TLS 私钥路径', hint: 'PEM 私钥文件路径', fullWidth: true },
      { key: 'tls_ca_path', label: 'TLS CA 路径', hint: '可信 CA 文件路径', fullWidth: true },
      { key: 'tls_server_name', label: 'TLS 服务名称', hint: '证书校验使用的服务名称' },
      { key: 'tls_allow_test_certificate', label: '允许测试证书', kind: 'boolean', hint: '仅用于测试环境' },
      { key: 'tls_insecure_skip_verify', label: '跳过证书校验', kind: 'boolean', hint: '高风险，仅用于隔离测试环境' },
    ],
  },
  {
    key: 'cluster', label: '节点运行', description: 'UDP 工作线程、套接字缓冲与节点密钥。', fields: [
      { key: 'udp_workers_auto', label: '自动分配 UDP Worker', kind: 'boolean', hint: '按 CPU 核心数决定工作线程' },
      { key: 'udp_workers', label: 'UDP Worker 数量', kind: 'number', hint: '关闭自动分配时生效' },
      { key: 'udp_receive_buffer_bytes', label: 'UDP 接收缓冲区', kind: 'number', hint: '单位：字节' },
      { key: 'udp_send_buffer_bytes', label: 'UDP 发送缓冲区', kind: 'number', hint: '单位：字节' },
      { key: 'secret_key', label: '节点密钥', kind: 'secret', hint: '留空表示不修改现有密钥', fullWidth: true },
    ],
  },
];

export function SettingsPage() {
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [configValues, setConfigValues] = useState<Entity>({});

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const result = await api.get<{ values: Entity }>('/infrastructure/settings');
      const configs = result.values.configs;
      setConfigValues(configs && typeof configs === 'object' ? (configs as Entity) : result.values);
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const updateValue = (key: string, value: unknown) =>
    setConfigValues((current) => ({ ...current, [key]: value }));

  const save = async () => {
    try {
      const payload = Object.fromEntries(
        Object.entries(configValues)
          .filter(([key, value]) => value !== undefined && value !== null && !(key === 'secret_key' && !value))
          .map(([key, value]) => [key, String(value)])
      );
      setSaving(true);
      await api.post('/infrastructure/settings', payload);
      message.success('设置已成功保存，节点重启后即刻生效');
    } catch (e) {
      if (e instanceof Error) message.error(e.message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-6 w-full">
      <Card shadow="sm" className="p-2 w-full">
        <CardBody className="p-4 flex flex-wrap items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-2 mb-1">
              <h2 className="text-base font-bold text-foreground">核心运行参数设置</h2>
              <Chip color="warning" size="sm" variant="flat">重启节点后生效</Chip>
            </div>
            <p className="text-tiny text-default-500">调整信令认证域、会话定时器、媒体 QoS、SBC 安全阀值与并发配额</p>
          </div>
          <div className="flex items-center gap-2">
            <Button variant="flat" size="sm" isLoading={loading} onPress={load} startContent={<RefreshCw className="w-4 h-4" />}>
              重新加载
            </Button>
            <Button color="primary" size="sm" isLoading={saving} onPress={save}>
              保存系统设置
            </Button>
          </div>
        </CardBody>
      </Card>

      {error ? (
        <ErrorState error={error} retry={load} />
      ) : loading ? (
        <div className="py-20 flex justify-center w-full">
          <Spinner color="primary" label="正在拉取核心节点参数..." />
        </div>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 w-full">
          {systemConfigGroups.map((group) => (
            <Card key={group.key} shadow="sm" className="p-2 w-full">
              <CardBody className="p-4 flex flex-col gap-3">
                <div className="border-b border-divider pb-2.5 flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <h3 className="text-sm font-bold text-foreground truncate">{group.label}</h3>
                    <p className="text-tiny text-default-400 mt-0.5 truncate">{group.description}</p>
                  </div>
                  <Chip size="sm" variant="bordered" className="text-default-500 text-tiny shrink-0">
                    {group.key.toUpperCase()}
                  </Chip>
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  {group.fields.map((field) => {
                    const value = configValues[field.key];
                    const enabled = String(value) === 'true' || value === true;
                    return (
                      <div key={field.key} className={`flex flex-col gap-1.5 ${field.fullWidth ? 'sm:col-span-2' : ''}`}>
                        <div className="flex items-center justify-between gap-2 text-small">
                          <span className="font-semibold text-default-700">{field.label}</span>
                        </div>

                        {field.kind === 'boolean' ? (
                          <Switch
                            size="sm"
                            color="success"
                            isSelected={enabled}
                            onValueChange={(val) => updateValue(field.key, val)}
                          >
                            <span className="text-tiny font-semibold text-default-500">
                              {enabled ? '已开启 (Enabled)' : '已停用 (Disabled)'}
                            </span>
                          </Switch>
                        ) : field.kind === 'secret' ? (
                          <Input
                            size="sm"
                            variant="bordered"
                            type="password"
                            placeholder={field.hint}
                            value={value !== undefined && value !== null ? String(value) : ''}
                            onValueChange={(val) => updateValue(field.key, val)}
                          />
                        ) : (
                          <Input
                            size="sm"
                            variant="bordered"
                            placeholder={field.hint}
                            value={value !== undefined && value !== null ? String(value) : ''}
                            onValueChange={(val) => updateValue(field.key, val)}
                          />
                        )}
                        <span className="text-tiny text-default-400">{field.hint}</span>
                      </div>
                    );
                  })}
                </div>
              </CardBody>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
