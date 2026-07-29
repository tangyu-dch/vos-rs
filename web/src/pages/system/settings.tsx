// 系统管理 - 核心运行参数设置
// 从 console.tsx 拆分

import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Card, CardBody, Input, Switch, Spinner } from '@heroui/react';
import { RefreshCw, Save, Settings2 } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import type { Entity } from '@/services/resources';
import { useAuth } from '@/auth/AuthContext';
import { hasPermission } from '@/services/auth';

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
interface SettingsSaveResult {
  values: {
    hot_reload_applied: string[];
    hot_reload_error?: string | null;
  };
  restart_required: boolean;
}

const systemConfigGroups: ConfigGroup[] = [
  {
    key: 'sip',
    label: '信令会话',
    description: '认证域与呼叫会话计时器。',
    fields: [
      { key: 'realm', label: '认证域', hint: '摘要认证使用的域；存在分机时不可修改' },
      { key: 'session_expires_gateway', label: '网关会话时长', kind: 'number', hint: '单位：秒' },
      { key: 'session_expires_caller', label: '主叫会话时长', kind: 'number', hint: '单位：秒' },
    ],
  },
  {
    key: 'routing',
    label: '中继路由',
    description: '路由运行依赖的中继健康探测。',
    fields: [
      {
        key: 'gateway_health_checks_enabled',
        label: '中继健康检查',
        kind: 'boolean',
        hint: '定期探测中继可用状态',
      },
    ],
  },
  {
    key: 'media',
    label: '媒体处理',
    description: '媒体地址学习、防欺骗与质量指标。',
    fields: [
      {
        key: 'rtp_symmetric_learning',
        label: '对称地址学习',
        kind: 'boolean',
        hint: '从首个有效媒体包学习源地址',
      },
      { key: 'rtp_anti_spoofing', label: '媒体防欺骗', kind: 'boolean', hint: '拒绝非预期媒体源' },
      {
        key: 'rtp_source_relearn_secs',
        label: '媒体源重新学习窗口',
        kind: 'number',
        hint: '单位：秒',
      },
      {
        key: 'media_metrics_log',
        label: '媒体指标日志',
        kind: 'boolean',
        hint: '输出媒体质量统计日志',
      },
    ],
  },
  {
    key: 'recording',
    label: '录音存储',
    description: '录音任务、存储容量与文件生命周期。',
    fields: [
      {
        key: 'recording_enabled',
        label: '启用录音',
        kind: 'boolean',
        hint: '允许系统创建通话录音',
      },
      { key: 'recording_dir', label: '录音目录', hint: '节点本地录音文件根目录', fullWidth: true },
      {
        key: 'recording_workers',
        label: '录音工作线程',
        kind: 'number',
        hint: '异步落盘工作线程数',
      },
      { key: 'recording_queue_capacity', label: '录音队列容量', hint: '等待写入的任务上限' },
      { key: 'recording_retention_secs', label: '录音保留时长', kind: 'number', hint: '单位：秒' },
      {
        key: 'recording_min_free_bytes',
        label: '最小磁盘余量',
        kind: 'number',
        hint: '单位：字节',
      },
      { key: 'recording_max_file_bytes', label: '单文件上限', kind: 'number', hint: '单位：字节' },
      {
        key: 'recording_max_duration_secs',
        label: '单次录音时长上限',
        kind: 'number',
        hint: '单位：秒',
      },
    ],
  },
  {
    key: 'billing',
    label: '计费话单',
    description: '余额风控、结算与话单持久化。',
    fields: [
      {
        key: 'balance_enforcement_enabled',
        label: '余额强制校验',
        kind: 'boolean',
        hint: '呼叫前校验账户可用余额',
      },
      {
        key: 'billing_settlement_enabled',
        label: '启用计费结算',
        kind: 'boolean',
        hint: '通话结束后执行费用结算',
      },
      {
        key: 'cdr_persistence_enabled',
        label: '话单持久化',
        kind: 'boolean',
        hint: '写入通话详单存储',
      },
      {
        key: 'cdr_queue_capacity',
        label: '话单队列容量',
        kind: 'number',
        hint: '等待持久化的话单上限',
      },
    ],
  },
  {
    key: 'security',
    label: '安全传输',
    description: '边界限流及信令加密连接安全。',
    fields: [
      {
        key: 'sbc_rate_limit_capacity',
        label: '令牌桶容量',
        kind: 'decimal',
        hint: '单一来源允许的突发请求量',
      },
      {
        key: 'sbc_rate_limit_fill_rate',
        label: '令牌补充速率',
        kind: 'decimal',
        hint: '每秒补充令牌数',
      },
      {
        key: 'sbc_max_concurrency',
        label: '边界最大并发',
        kind: 'number',
        hint: '边界层并发会话上限',
      },
      { key: 'tls_bind_addr', label: '加密监听地址', hint: '例如 0.0.0.0:5061' },
      { key: 'tls_cert_path', label: '证书路径', hint: '证书文件路径', fullWidth: true },
      { key: 'tls_key_path', label: '私钥路径', hint: '私钥文件路径', fullWidth: true },
      { key: 'tls_ca_path', label: '签发链路径', hint: '可信签发链文件路径', fullWidth: true },
      { key: 'tls_server_name', label: '服务名称', hint: '证书校验使用的服务名称' },
      {
        key: 'tls_allow_test_certificate',
        label: '允许测试证书',
        kind: 'boolean',
        hint: '仅用于测试环境',
      },
      {
        key: 'tls_insecure_skip_verify',
        label: '跳过证书校验',
        kind: 'boolean',
        hint: '高风险，仅用于隔离测试环境',
      },
    ],
  },
  {
    key: 'cluster',
    label: '节点运行',
    description: '数据报工作线程、套接字缓冲与节点密钥。',
    fields: [
      {
        key: 'udp_workers_auto',
        label: '自动分配线程',
        kind: 'boolean',
        hint: '按处理器核心数决定工作线程',
      },
      { key: 'udp_workers', label: '数据报线程数量', kind: 'number', hint: '关闭自动分配时生效' },
      {
        key: 'udp_receive_buffer_bytes',
        label: '数据报接收缓冲',
        kind: 'number',
        hint: '单位：字节',
      },
      { key: 'udp_send_buffer_bytes', label: '数据报发送缓冲', kind: 'number', hint: '单位：字节' },
      {
        key: 'secret_key',
        label: '节点密钥',
        kind: 'secret',
        hint: '留空表示不修改现有密钥',
        fullWidth: true,
      },
    ],
  },
];

const editableSystemConfigKeys = new Set(
  systemConfigGroups.flatMap((group) => group.fields.map((field) => field.key)),
);

export function SettingsPage() {
  const { session } = useAuth();
  const canManage = Boolean(session && hasPermission(session, 'settings.manage'));
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [configValues, setConfigValues] = useState<Entity>({});
  const loadedConfigValues = useRef<Entity>({});

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const result = await api.get<{ values: Entity }>('/infrastructure/settings');
      const configs = result.values.configs;
      const loadedValues =
        configs && typeof configs === 'object' ? (configs as Entity) : result.values;
      const editableValues = Object.fromEntries(
        Object.entries(loadedValues).filter(([key]) => editableSystemConfigKeys.has(key)),
      );
      loadedConfigValues.current = editableValues;
      setConfigValues(editableValues);
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const updateValue = (key: string, value: unknown) =>
    setConfigValues((current) => ({ ...current, [key]: value }));

  const save = async () => {
    try {
      const payload = Object.fromEntries(
        Object.entries(configValues)
          .filter(
            ([key, value]) =>
              editableSystemConfigKeys.has(key) &&
              value !== undefined &&
              value !== null &&
              !(key === 'secret_key' && !value) &&
              String(value) !== String(loadedConfigValues.current[key] ?? ''),
          )
          .map(([key, value]) => [key, String(value)]),
      );
      if (Object.keys(payload).length === 0) {
        message.info('配置没有变化');
        return;
      }
      setSaving(true);
      const result = await api.post<SettingsSaveResult>('/infrastructure/settings', payload);
      loadedConfigValues.current = { ...loadedConfigValues.current, ...payload };
      if (result.values.hot_reload_error) {
        message.warning(`配置已保存；${result.values.hot_reload_error}`);
      } else if (result.values.hot_reload_applied.length > 0 && result.restart_required) {
        message.success('录音运行配置已热更新，新通话立即生效；其余配置重启节点后生效');
      } else if (result.values.hot_reload_applied.length > 0) {
        message.success('配置已热更新，新通话立即生效');
      } else {
        message.success('配置已保存，重启相关节点后生效');
      }
    } catch (e) {
      if (e instanceof Error) message.error(e.message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-4 w-full">
      <Card shadow="none" className="w-full border border-default-200 bg-content1 shadow-sm">
        <CardBody className="flex flex-row flex-wrap items-center justify-between gap-4 p-5">
          <div>
            <h1 className="flex items-center gap-2 text-lg font-semibold text-foreground">
              <Settings2 className="h-5 w-5 text-primary" />
              系统设置
            </h1>
            <p className="mt-1 text-small text-default-500">
              录音开关与安全限制保存后热更新，其余配置重启相关节点后生效
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Button
              isIconOnly
              variant="flat"
              isLoading={loading}
              onPress={load}
              aria-label="刷新系统设置"
            >
              <RefreshCw className="h-4 w-4" />
            </Button>
            <Button
              color="primary"
              isLoading={saving}
              isDisabled={!canManage}
              onPress={save}
              startContent={<Save className="h-4 w-4" />}
            >
              保存设置
            </Button>
          </div>
        </CardBody>
      </Card>

      {error ? (
        <ErrorState error={error} retry={load} />
      ) : loading ? (
        <div className="py-20 flex justify-center w-full">
          <Spinner color="primary" label="正在加载系统设置" />
        </div>
      ) : (
        <div className="w-full columns-1 gap-4 lg:columns-2">
          {systemConfigGroups.map((group) => (
            <Card
              key={group.key}
              shadow="none"
              className="mb-4 inline-block w-full break-inside-avoid border border-default-200 bg-content1 shadow-sm"
            >
              <CardBody className="flex flex-col gap-3 p-5">
                <div className="border-b border-divider pb-2.5 flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <h3 className="text-sm font-medium text-foreground">{group.label}</h3>
                    <p className="text-tiny text-default-500 mt-1">{group.description}</p>
                  </div>
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  {group.fields.map((field) => {
                    const value = configValues[field.key];
                    const enabled = String(value) === 'true' || value === true;
                    return (
                      <div
                        key={field.key}
                        className={`flex flex-col gap-1.5 ${field.fullWidth ? 'sm:col-span-2' : ''}`}
                      >
                        <div className="flex items-center justify-between gap-2 text-small">
                          <span className="font-medium text-default-700">{field.label}</span>
                        </div>

                        {field.kind === 'boolean' ? (
                          <Switch
                            size="sm"
                            color="primary"
                            isSelected={enabled}
                            isDisabled={!canManage}
                            onValueChange={(val) => updateValue(field.key, val)}
                          >
                            <span className="text-tiny font-normal text-default-500">
                              {enabled ? '已开启' : '已停用'}
                            </span>
                          </Switch>
                        ) : field.kind === 'secret' ? (
                          <Input
                            size="sm"
                            variant="bordered"
                            type="password"
                            isDisabled={!canManage}
                            placeholder={field.hint}
                            value={value !== undefined && value !== null ? String(value) : ''}
                            onValueChange={(val) => updateValue(field.key, val)}
                          />
                        ) : (
                          <Input
                            size="sm"
                            variant="bordered"
                            placeholder={field.hint}
                            isDisabled={!canManage}
                            value={value !== undefined && value !== null ? String(value) : ''}
                            onValueChange={(val) => updateValue(field.key, val)}
                          />
                        )}
                        <span className="text-tiny leading-5 text-default-500">{field.hint}</span>
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
