import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Button,
  Tag,
  Message,
  Popconfirm,
  Alert,
  Empty,
} from '@arco-design/web-react';
import { IconRefresh, IconDelete } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import { useAuth } from '@/auth/AuthContext';
import type { ActiveCall } from '@/types';
import { extractSipUser } from '@/utils/sip';
import { usePageVisibility } from '@/hooks/usePageVisibility';

const STATE_MAP: Record<string, { color: string; text: string }> = {
  Routing: { color: 'blue', text: '路由中' },
  Ringing: { color: 'orange', text: '振铃中' },
  Established: { color: 'green', text: '通话中' },
};

function duration(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(s / 60);
  if (m > 0) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

export default function ActiveCalls() {
  const { session } = useAuth();
  const [calls, setCalls] = useState<ActiveCall[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);
  const pageVisible = usePageVisibility();

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setCalls(await apiService.getActiveCalls());
      setNow(Date.now());
      setLastUpdated(Date.now());
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!pageVisible) return;
    void load();
    const t = setInterval(load, 5000);
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      clearInterval(t);
      clearInterval(tick);
    };
  }, [load, pageVisible]);

  const handleTerminate = async (callId: string) => {
    try {
      await apiService.terminateCall(callId);
      Message.success('已发送强制拆线');
      load();
    } catch {
      Message.error('拆线失败');
    }
  };

  const columns = [
    {
      title: 'Call ID',
      dataIndex: 'call_id',
      render: (v: string) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '主叫',
      dataIndex: 'caller',
      render: (c: string) => <span className="cell-mono">{extractSipUser(c)}</span>,
    },
    {
      title: '被叫',
      dataIndex: 'callee',
      render: (c: string) => <span className="cell-mono">{extractSipUser(c)}</span>,
    },
    {
      title: '状态',
      dataIndex: 'state',
      width: 110,
      render: (s: string) => {
        const m = STATE_MAP[s] || { color: 'gray', text: s };
        return <Tag color={m.color}>{m.text}</Tag>;
      },
    },
    {
      title: '通话时长',
      dataIndex: 'started_at_ms',
      width: 120,
      render: (ms: number) => <span className="cell-mono">{duration(now - ms)}</span>,
    },
    {
      title: '网关',
      dataIndex: 'gateway',
      render: (g: string) => (g ? <span className="cell-mono">{g}</span> : '—'),
    },
    {
      title: '操作',
      dataIndex: 'actions',
      width: 100,
      fixed: 'right' as const,
      render: (_: any, r: ActiveCall) => session?.role === 'financier' ? null : (
        <Popconfirm title="确认强制拆线？" icon={null} onOk={() => handleTerminate(r.call_id)}>
          <Button type="text" size="small" status="danger" icon={<IconDelete />}>
            拆线
          </Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>活跃呼叫</h1>
          <span className="sub">实时通话监控（每 5 秒刷新，可强制拆线）</span>
        </div>
        <div className="page-header__actions">
          <span className={`sync-status ${error ? 'sync-status--error' : 'sync-status--online'}`}>
            <span className="sync-status__dot" />
            {error ? '连接异常' : lastUpdated ? `已同步 ${new Date(lastUpdated).toLocaleTimeString('zh-CN')}` : '正在连接'}
          </span>
          <Button icon={<IconRefresh />} onClick={load}>
            刷新
          </Button>
        </div>
      </div>

      <div className="reg-stats">
        <div className="reg-stats__item">
          <span className="reg-stats__num font-num">{calls.length}</span>
          <span className="reg-stats__label">当前活跃呼叫</span>
        </div>
      </div>

      {error && (
        <Alert
          type="error"
          content={`获取活跃呼叫失败（sip-edge 管理 API 未启用？）：${error}`}
          closable
          style={{ marginBottom: 16 }}
        />
      )}

      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={calls}
          rowKey="call_id"
          loading={loading}
          pagination={false}
          noDataElement={<Empty description="当前无活跃呼叫" />}
        />
      </Card>
    </div>
  );
}
