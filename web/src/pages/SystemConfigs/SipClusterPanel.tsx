import { useCallback, useEffect, useState } from 'react';
import { Alert, Button, Message, Space, Spin, Table, Tag } from '@arco-design/web-react';
import { IconRefresh } from '@arco-design/web-react/icon';
import { apiService, type SipClusterNodeStatus, type SipClusterStatus } from '@/services/api';

const MODE_LABELS: Record<SipClusterNodeStatus['router_mode'], string> = {
  direct: '直接接入',
  external: '外部代理',
  native: '原生路由器',
};

const STATUS_LABELS: Record<SipClusterNodeStatus['status'], string> = {
  active: '接收新呼叫',
  draining: '摘流中',
};

export default function SipClusterPanel() {
  const [status, setStatus] = useState<SipClusterStatus>();
  const [loading, setLoading] = useState(false);
  const [changingNode, setChangingNode] = useState<string>();

  const load = useCallback(async (notify = false) => {
    setLoading(true);
    try {
      setStatus(await apiService.getSipClusterStatus());
      if (notify) Message.success('SIP 集群状态已刷新');
    } catch (error) {
      Message.error(error instanceof Error ? error.message : '加载 SIP 集群状态失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const controlNode = async (node: SipClusterNodeStatus) => {
    const action = node.status === 'active' ? 'drain' : 'resume';
    setChangingNode(node.node_id);
    try {
      const result = await apiService.controlSipClusterNode(node.node_id, action);
      setStatus((current) => current && ({
        ...current,
        active_nodes: current.active_nodes + (result.status === 'active' ? 1 : -1),
        draining_nodes: current.draining_nodes + (result.status === 'draining' ? 1 : -1),
        nodes: current.nodes.map((item) => item.node_id === node.node_id
          ? { ...item, status: result.status, active_calls: result.active_calls }
          : item),
      }));
      Message.success(action === 'drain' ? '节点已摘流，新呼叫将不再分配到该节点' : '节点已恢复接收新呼叫');
      window.setTimeout(() => { void load(); }, 1500);
    } catch (error) {
      Message.error(error instanceof Error ? error.message : '修改节点状态失败');
    } finally {
      setChangingNode(undefined);
    }
  };

  return (
    <Spin loading={loading} style={{ width: '100%' }}>
      <section className="system-configs__panel">
        <div className="system-configs__cluster-header">
          <div className="system-configs__intro">
            <h2>SIP Edge 在线节点</h2>
            <p>节点通过 Redis TTL 心跳自动注册；原生 sip-router 只会调度“原生路由器”模式的节点。</p>
          </div>
          <Button icon={<IconRefresh />} onClick={() => void load(true)}>刷新状态</Button>
        </div>
        <Alert
          className="system-configs__notice"
          type="info"
          showIcon
          content="节点标识、监听地址和路由模式属于每台服务器的引导配置，必须分别写入各节点 config.yaml；这里展示运行时状态，避免将同一份全局配置错误下发到所有节点。"
        />
        <div className="system-configs__summary">
          <Tag color={status?.online_nodes ? 'green' : 'red'}>在线节点 {status?.online_nodes ?? 0}</Tag>
          <Tag color="green">活动节点 {status?.active_nodes ?? 0}</Tag>
          <Tag color={status?.draining_nodes ? 'orange' : 'gray'}>摘流节点 {status?.draining_nodes ?? 0}</Tag>
          <Tag color="arcoblue">心跳前缀 {status?.node_key_prefix ?? '-'}</Tag>
        </div>
        <Table<SipClusterNodeStatus>
          rowKey="node_id"
          pagination={false}
          data={status?.nodes ?? []}
          noDataElement="未发现在线 SIP 节点，请检查 cluster.enabled、Redis 和节点心跳配置"
          columns={[
            { title: '节点标识', dataIndex: 'node_id' },
            { title: '通告地址', dataIndex: 'advertised_addr' },
            { title: '接入模式', dataIndex: 'router_mode', render: (value) => <Tag color={value === 'native' ? 'green' : 'orange'}>{MODE_LABELS[value as SipClusterNodeStatus['router_mode']] ?? value}</Tag> },
            { title: '运行状态', dataIndex: 'status', render: (value) => <Tag color={value === 'active' ? 'green' : 'orange'}>{STATUS_LABELS[value as SipClusterNodeStatus['status']] ?? value}</Tag> },
            { title: '活动呼叫', dataIndex: 'active_calls' },
            { title: '版本', dataIndex: 'version', render: (value) => value || '-' },
            { title: '剩余 TTL', dataIndex: 'ttl_secs', render: (value) => `${value} 秒` },
            { title: '最后心跳', dataIndex: 'updated_at', render: (value) => new Date(Number(value) * 1000).toLocaleString('zh-CN') },
            {
              title: '操作',
              render: (_, node) => (
                <Space>
                  <Button
                    size="small"
                    status={node.status === 'active' ? 'danger' : 'success'}
                    loading={changingNode === node.node_id}
                    disabled={!node.management_url || (Boolean(changingNode) && changingNode !== node.node_id)}
                    onClick={() => void controlNode(node)}
                  >
                    {node.status === 'active' ? '摘流' : '恢复'}
                  </Button>
                </Space>
              ),
            },
          ]}
        />
      </section>
    </Spin>
  );
}
