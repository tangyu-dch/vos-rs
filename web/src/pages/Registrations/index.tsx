import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Button,
  Input,
  Message,
  Alert,
  Empty,
} from '@arco-design/web-react';
import { IconRefresh, IconSearch } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { SipRegistration } from '@/types';

function getExpStatus(expiresAt: string) {
  const diffMs = new Date(expiresAt).getTime() - Date.now();
  const mins = Math.floor(diffMs / 60000);
  if (mins < 0) return { text: '已过期', cls: 'status-tag status-tag--failed' };
  if (mins < 5) return { text: '即将过期', cls: 'status-tag status-tag--canceled' };
  return { text: '在线', cls: 'status-tag status-tag--answered' };
}

export default function Registrations() {
  const [registrations, setRegistrations] = useState<SipRegistration[]>([]);
  const [filtered, setFiltered] = useState<SipRegistration[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchText, setSearchText] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiService.getRegistrations();
      setRegistrations(data);
      setFiltered(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取注册信息失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    const kw = searchText.trim().toLowerCase();
    if (!kw) {
      setFiltered(registrations);
      return;
    }
    setFiltered(
      registrations.filter(
        (r) =>
          r.aor.toLowerCase().includes(kw) ||
          r.contact_uri.toLowerCase().includes(kw) ||
          r.received_from.toLowerCase().includes(kw)
      )
    );
  }, [searchText, registrations]);

  const onlineCount = registrations.filter(
    (r) => new Date(r.expires_at).getTime() > Date.now()
  ).length;

  const columns = [
    {
      title: 'AOR',
      dataIndex: 'aor',
      render: (v: string) => <span className="cell-mono cell-strong">{v}</span>,
    },
    {
      title: '联系地址',
      dataIndex: 'contact_uri',
      render: (v: string) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '来源地址',
      dataIndex: 'received_from',
      render: (v: string) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '状态',
      dataIndex: 'expires_at',
      width: 110,
      render: (v: string) => {
        const s = getExpStatus(v);
        return <span className={s.cls}>{s.text}</span>;
      },
    },
    {
      title: '过期时间',
      dataIndex: 'expires_at',
      width: 170,
      render: (v: string) => new Date(v).toLocaleString('zh-CN'),
    },
    {
      title: 'Path',
      dataIndex: 'path',
      width: 160,
      render: (p: string[]) => (p && p.length ? p.join(', ') : '—'),
    },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>注册信息</h1>
          <span className="sub">查看当前在线终端的 SIP 注册状态</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={load}>
            刷新
          </Button>
        </div>
      </div>

      <div className="reg-stats">
        <div className="reg-stats__item">
          <span className="reg-stats__num font-num">{onlineCount}</span>
          <span className="reg-stats__label">在线终端</span>
        </div>
        <div className="reg-stats__item">
          <span className="reg-stats__num font-num">{registrations.length}</span>
          <span className="reg-stats__label">注册记录</span>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false}>
        <div className="table-toolbar">
          <Input
            placeholder="搜索 AOR / 联系地址 / 来源"
            style={{ width: 320 }}
            value={searchText}
            onChange={setSearchText}
            prefix={<IconSearch />}
            allowClear
          />
        </div>
        <Table
          className="app-table"
          columns={columns}
          data={filtered}
          rowKey={(r) => `${r.aor}-${r.contact_uri}`}
          loading={loading}
          pagination={{ pageSize: 20, sizeCanChange: true }}
          scroll={{ x: 980 }}
          noDataElement={<Empty description="暂无注册记录" />}
        />
      </Card>
    </div>
  );
}
