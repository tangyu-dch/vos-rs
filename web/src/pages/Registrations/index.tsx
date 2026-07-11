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
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchText, setSearchText] = useState('');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [total, setTotal] = useState(0);

  const load = useCallback(async (nextPage = 1, nextPageSize = 20, keyword = '') => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiService.getRegistrations(nextPage, nextPageSize, keyword || undefined);
      setRegistrations(data.items);
      setTotal(data.total);
      setPage(nextPage);
      setPageSize(nextPageSize);
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

  // 后端只返回 expires_at > now() 的有效注册，total 即在线终端总数。
  const onlineCount = total;

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
          <Button icon={<IconRefresh />} onClick={() => load(page, pageSize, searchText)}>
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
          <span className="reg-stats__num font-num">{total}</span>
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
            onChange={(value) => {
              setSearchText(value);
              load(1, pageSize, value);
            }}
            prefix={<IconSearch />}
            allowClear
          />
        </div>
        <Table
          className="app-table"
          columns={columns}
          data={registrations}
          rowKey={(r) => `${r.aor}-${r.contact_uri}`}
          loading={loading}
          pagination={{ current: page, pageSize, total, sizeCanChange: true, sizeOptions: [10, 20, 50, 100], onChange: (nextPage) => load(nextPage, pageSize, searchText), onPageSizeChange: (nextPageSize) => load(1, nextPageSize, searchText) }}
          scroll={{ x: 980 }}
          noDataElement={<Empty description="暂无注册记录" />}
        />
      </Card>
    </div>
  );
}
