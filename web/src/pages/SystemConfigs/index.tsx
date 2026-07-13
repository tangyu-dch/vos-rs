import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Alert,
  Button,
  Card,
  Form,
  Input,
  InputNumber,
  Message,
  Space,
  Spin,
  Switch,
  Tabs,
  Tag,
} from '@arco-design/web-react';
import { IconRefresh, IconSave } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import { CONFIG_FIELD_MAP, CONFIG_FIELDS, CONFIG_GROUPS, type ConfigField } from './configSchema';
import './SystemConfigs.css';

type FormValues = Record<string, boolean | number | string>;

function parseConfigValues(configs: Record<string, string>): FormValues {
  const values: FormValues = {};
  for (const field of CONFIG_FIELDS) {
    const raw = configs[field.key];
    if (raw === undefined) continue;
    if (field.type === 'boolean') values[field.key] = raw === 'true' || raw === '1';
    else if (field.type === 'integer' || field.type === 'number') values[field.key] = Number(raw);
    else values[field.key] = raw;
  }
  return values;
}

function serializeConfigValues(values: FormValues): Record<string, string> {
  return Object.fromEntries(
    Object.entries(values)
      .filter(([key, value]) => CONFIG_FIELD_MAP.has(key) && value !== undefined && value !== null)
      .filter(([key, value]) => CONFIG_FIELD_MAP.get(key)?.type !== 'secret' || String(value).trim() !== '')
      .map(([key, value]) => [key, String(value)]),
  );
}

function ConfigControl({ field }: { field: ConfigField }) {
  if (field.type === 'boolean') {
    return <div className="system-configs__switch"><Switch /></div>;
  }
  if (field.type === 'integer' || field.type === 'number') {
    return <InputNumber min={field.min} max={field.max} step={field.step} precision={field.type === 'integer' ? 0 : undefined} style={{ width: '100%' }} />;
  }
  if (field.type === 'secret') return <Input.Password placeholder={field.placeholder ?? '输入新值'} />;
  return <Input placeholder={field.placeholder} allowClear />;
}

function fieldRules(field: ConfigField) {
  if (field.type === 'boolean' || field.type === 'secret') return undefined;
  return [{ required: !['tls_bind_addr', 'tls_cert_path', 'tls_key_path', 'tls_ca_path', 'tls_server_name'].includes(field.key), message: `请填写${field.label}` }];
}

export default function SystemConfigs() {
  const [form] = Form.useForm<FormValues>();
  const [loading, setLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [loadedValues, setLoadedValues] = useState<FormValues>({});

  const loadConfigs = useCallback(async (showSuccess = false) => {
    setLoading(true);
    try {
      const values = parseConfigValues(await apiService.getSystemConfigs());
      form.setFieldsValue(values);
      setLoadedValues(values);
      if (showSuccess) Message.success('系统配置已刷新');
    } catch (error) {
      Message.error(error instanceof Error ? error.message : '加载配置失败');
    } finally {
      setLoading(false);
    }
  }, [form]);

  useEffect(() => { void loadConfigs(); }, [loadConfigs]);

  const configuredCount = useMemo(() => Object.keys(loadedValues).length, [loadedValues]);

  const handleSubmit = async (values: FormValues) => {
    if (Number(values.rtp_port_min) >= Number(values.rtp_port_max)) {
      Message.error('RTP 最大端口必须大于最小端口');
      return;
    }
    setSubmitting(true);
    try {
      const payload = serializeConfigValues(values);
      await apiService.updateSystemConfigs(payload);
      setLoadedValues(values);
      Message.success(`已保存 ${Object.keys(payload).length} 项配置，重启 sip-edge 后生效`);
    } catch (error) {
      Message.error(error instanceof Error ? error.message : '保存配置失败');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>系统参数配置</h1>
          <span className="sub">集中管理 Redis 动态配置，并持久化至 PostgreSQL</span>
        </div>
        <div className="page-header__actions">
          <Space>
            <Button icon={<IconRefresh />} onClick={() => void loadConfigs(true)} disabled={loading || submitting}>刷新</Button>
            <Button type="primary" icon={<IconSave />} loading={submitting} onClick={() => form.submit()}>保存配置</Button>
          </Space>
        </div>
      </div>

      <Alert
        className="system-configs__notice"
        type="warning"
        showIcon
        content="保存操作会同步写入 PostgreSQL 和 Redis。当前 sip-edge 在启动时加载这些参数，因此需要滚动重启 sip-edge 才会生效。数据库、Redis、NATS、监听地址和管理密钥属于引导配置，只能在 config.yaml 中维护。"
      />
      <div className="system-configs__summary">
        <Tag color="arcoblue">{CONFIG_GROUPS.length} 个配置分组</Tag>
        <Tag color="green">已加载 {configuredCount}/{CONFIG_FIELDS.length} 项</Tag>
        <Tag color="orange">仅管理员可修改</Tag>
      </div>

      <Spin loading={loading} style={{ width: '100%' }}>
        <Card className="form-card" bordered={false}>
          <Form form={form} layout="vertical" onSubmit={handleSubmit} className="config-form">
            <Tabs defaultActiveTab="sip" type="rounded">
              {CONFIG_GROUPS.map((group) => (
                <Tabs.TabPane key={group.key} title={group.title}>
                  <section className="system-configs__panel">
                    <div className="system-configs__intro">
                      <h2>{group.title}</h2>
                      <p>{group.description}</p>
                    </div>
                    <div className="system-configs__grid">
                      {group.fields.map((field) => (
                        <Form.Item
                          key={field.key}
                          className={field.type === 'string' || field.type === 'secret' ? 'system-configs__field--wide' : undefined}
                          field={field.key}
                          label={field.label}
                          triggerPropName={field.type === 'boolean' ? 'checked' : undefined}
                          rules={fieldRules(field)}
                        >
                          <ConfigControl field={field} />
                          <span className="system-configs__field-help">{field.description}</span>
                        </Form.Item>
                      ))}
                    </div>
                  </section>
                </Tabs.TabPane>
              ))}
            </Tabs>
          </Form>
        </Card>
      </Spin>
    </div>
  );
}
