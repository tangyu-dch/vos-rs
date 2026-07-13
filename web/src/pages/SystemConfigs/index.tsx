import { useState, useEffect } from 'react';
import {
  Card,
  Form,
  Input,
  InputNumber,
  Switch,
  Button,
  Tabs,
  Message,
  Spin,
  Space,
} from '@arco-design/web-react';
import { IconSave, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';

const TabPane = Tabs.TabPane;

export default function SystemConfigs() {
  const [form] = Form.useForm();
  const [loading, setLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const loadConfigs = async () => {
    setLoading(true);
    try {
      const data = await apiService.getSystemConfigs();
      // Ensure boolean values are mapped properly to form switches
      const formattedData: Record<string, any> = {};
      for (const [key, val] of Object.entries(data)) {
        if (['rtp_symmetric_learning', 'rtp_anti_spoofing', 'recording_enabled'].includes(key)) {
          formattedData[key] = val === 'true' || val === '1';
        } else if ([
          'session_expires_gateway',
          'session_expires_caller',
          'rtp_port_min',
          'rtp_port_max',
          'rtp_source_relearn_secs',
          'recording_retention_secs',
          'recording_min_free_bytes',
          'recording_max_file_bytes',
          'recording_max_duration_secs',
          'sbc_max_concurrency'
        ].includes(key)) {
          formattedData[key] = Number(val);
        } else if (['sbc_rate_limit_capacity', 'sbc_rate_limit_fill_rate'].includes(key)) {
          formattedData[key] = parseFloat(val);
        } else {
          formattedData[key] = val;
        }
      }
      form.setFieldsValue(formattedData);
      Message.success('系统配置加载成功');
    } catch (err) {
      Message.error(err instanceof Error ? err.message : '加载配置失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadConfigs();
  }, []);

  const handleSubmit = async (values: Record<string, any>) => {
    setSubmitting(true);
    try {
      const payload: Record<string, string> = {};
      for (const [key, val] of Object.entries(values)) {
        if (val === undefined || val === null) {
          payload[key] = '';
        } else {
          payload[key] = String(val);
        }
      }
      await apiService.updateSystemConfigs(payload);
      Message.success('配置已保存成功，稍后网关将自动应用');
    } catch (err) {
      Message.error(err instanceof Error ? err.message : '保存配置失败');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>系统参数配置</h1>
          <span className="sub">管理电信级 VoIP 软交换平台的信令、媒体、SBC 限制与录音存储配置</span>
        </div>
        <div className="page-header__actions">
          <Space>
            <Button
              icon={<IconRefresh />}
              onClick={loadConfigs}
              disabled={loading || submitting}
            >
              刷新
            </Button>
            <Button
              type="primary"
              icon={<IconSave />}
              loading={submitting}
              onClick={() => form.submit()}
            >
              保存配置
            </Button>
          </Space>
        </div>
      </div>

      <Spin loading={loading} style={{ width: '100%' }}>
        <Card className="form-card" bordered={false}>
          <Form
            form={form}
            layout="vertical"
            onSubmit={handleSubmit}
            className="config-form"
          >
            <Tabs defaultActiveTab="sip" type="rounded">
              <TabPane key="sip" title="SIP 信令控制">
                <div style={{ padding: '16px 0', maxWidth: 640 }}>
                  <Form.Item
                    label="网关会话过期时间 (秒)"
                    field="session_expires_gateway"
                    rules={[{ required: true, message: '请输入网关会话过期时间' }]}
                    tooltip="对应 SIP Session Timer，用于网关中继方向的心跳检测周期"
                  >
                    <InputNumber min={60} max={86400} style={{ width: '100%' }} />
                  </Form.Item>

                  <Form.Item
                    label="终端会话过期时间 (秒)"
                    field="session_expires_caller"
                    rules={[{ required: true, message: '请输入终端会话过期时间' }]}
                    tooltip="主叫客户端注册的会话过期周期"
                  >
                    <InputNumber min={60} max={86400} style={{ width: '100%' }} />
                  </Form.Item>
                </div>
              </TabPane>

              <TabPane key="media" title="RTP 媒体服务">
                <div style={{ padding: '16px 0', maxWidth: 640 }}>
                  <Form.Item
                    label="媒体网关外网通告 IP (Advertised Addr)"
                    field="rtp_advertised_addr"
                    rules={[{ required: true, message: '请输入媒体通告 IP' }]}
                    tooltip="NAT 环境下映射 of 媒体网关外网公网 IP 地址"
                  >
                    <Input placeholder="例如: 47.96.12.34" />
                  </Form.Item>

                  <Space size="large" style={{ width: '100%', display: 'flex' }}>
                    <Form.Item
                      label="RTP 最小起始端口"
                      field="rtp_port_min"
                      rules={[{ required: true, message: '请输入最小端口' }]}
                      style={{ flex: 1 }}
                    >
                      <InputNumber min={1024} max={65535} />
                    </Form.Item>
                    <Form.Item
                      label="RTP 最大截止端口"
                      field="rtp_port_max"
                      rules={[{ required: true, message: '请输入最大端口' }]}
                      style={{ flex: 1 }}
                    >
                      <InputNumber min={1024} max={65535} />
                    </Form.Item>
                  </Space>

                  <Space size="large" style={{ width: '100%', display: 'flex', marginTop: 12 }}>
                    <Form.Item
                      label="对称 RTP 学习"
                      field="rtp_symmetric_learning"
                      triggerPropName="checked"
                    >
                      <Switch />
                    </Form.Item>
                    <Form.Item
                      label="防 IP 欺骗校验 (Anti-spoofing)"
                      field="rtp_anti_spoofing"
                      triggerPropName="checked"
                    >
                      <Switch />
                    </Form.Item>
                  </Space>

                  <Form.Item
                    label="RTP 路径重新学习延迟 (秒)"
                    field="rtp_source_relearn_secs"
                    rules={[{ required: true, message: '请输入路径重新学习延迟' }]}
                    style={{ marginTop: 12 }}
                  >
                    <InputNumber min={1} max={300} style={{ width: '100%' }} />
                  </Form.Item>
                </div>
              </TabPane>

              <TabPane key="recording" title="录音文件存储">
                <div style={{ padding: '16px 0', maxWidth: 640 }}>
                  <Form.Item
                    label="启用通话录音"
                    field="recording_enabled"
                    triggerPropName="checked"
                  >
                    <Switch />
                  </Form.Item>

                  <Form.Item
                    label="本地录音文件保存根目录"
                    field="recording_dir"
                    rules={[{ required: true, message: '请输入录音根目录' }]}
                  >
                    <Input placeholder="例如: /var/vos/recordings" />
                  </Form.Item>

                  <Form.Item
                    label="录音文件保留时长 (秒)"
                    field="recording_retention_secs"
                    rules={[{ required: true, message: '请输入保留天数（以秒为单位）' }]}
                  >
                    <InputNumber min={3600} style={{ width: '100%' }} />
                  </Form.Item>

                  <Space size="large" style={{ width: '100%', display: 'flex' }}>
                    <Form.Item
                      label="存储卷最小剩余可用空间 (Bytes)"
                      field="recording_min_free_bytes"
                      rules={[{ required: true, message: '请输入最小剩余可用空间' }]}
                      style={{ flex: 1 }}
                    >
                      <InputNumber min={0} />
                    </Form.Item>

                    <Form.Item
                      label="单个录音文件最大体积限制 (Bytes)"
                      field="recording_max_file_bytes"
                      rules={[{ required: true, message: '请输入最大体积限制' }]}
                      style={{ flex: 1 }}
                    >
                      <InputNumber min={0} />
                    </Form.Item>
                  </Space>

                  <Form.Item
                    label="单次通话最大录音时长限制 (秒)"
                    field="recording_max_duration_secs"
                    rules={[{ required: true, message: '请输入最大时长限制' }]}
                  >
                    <InputNumber min={10} style={{ width: '100%' }} />
                  </Form.Item>
                </div>
              </TabPane>

              <TabPane key="security" title="安全认证与 SBC">
                <div style={{ padding: '16px 0', maxWidth: 640 }}>
                  <Form.Item
                    label="SIP 挑战认证 Realm 域"
                    field="realm"
                    rules={[{ required: true, message: '请输入挑战 Realm 域' }]}
                  >
                    <Input placeholder="例如: vos-rs-auth" />
                  </Form.Item>

                  <Form.Item
                    label="SIP 挑战认证 Nonce"
                    field="nonce"
                    rules={[{ required: true, message: '请输入挑战 Nonce' }]}
                  >
                    <Input placeholder="例如: dynamic-vos-nonce" />
                  </Form.Item>

                  <Form.Item
                    label="安全签名秘钥 (Secret Key)"
                    field="secret_key"
                    rules={[{ required: true, message: '请输入安全签名秘钥' }]}
                  >
                    <Input.Password placeholder="安全私钥" />
                  </Form.Item>

                  <Space size="large" style={{ width: '100%', display: 'flex' }}>
                    <Form.Item
                      label="SBC 令牌桶容量 (Capacity)"
                      field="sbc_rate_limit_capacity"
                      rules={[{ required: true, message: '请输入 SBC 限制容量' }]}
                      style={{ flex: 1 }}
                    >
                      <InputNumber min={1.0} />
                    </Form.Item>

                    <Form.Item
                      label="SBC 令牌注入速率 (Fill Rate / 秒)"
                      field="sbc_rate_limit_fill_rate"
                      rules={[{ required: true, message: '请输入 SBC 注入速率' }]}
                      style={{ flex: 1 }}
                    >
                      <InputNumber min={1.0} />
                    </Form.Item>
                  </Space>

                  <Form.Item
                    label="系统最大全局并发通话量 (Max Concurrency)"
                    field="sbc_max_concurrency"
                    rules={[{ required: true, message: '请输入系统最大并发数' }]}
                  >
                    <InputNumber min={10} style={{ width: '100%' }} />
                  </Form.Item>
                </div>
              </TabPane>
            </Tabs>
          </Form>
        </Card>
      </Spin>
    </div>
  );
}
