import { useCallback, useEffect, useState } from 'react';
import { Alert, Card, Empty, Message, Table, Tag } from '@arco-design/web-react';
import { apiService } from '@/services/api';
import type { AuditLog } from '@/types';

const ROLE_LABEL: Record<string, string> = {
  admin: '管理员',
  operator: '运维',
  financier: '财务',
};

export default function AuditLogs() {
  const [logs, setLogs] = useState<AuditLog[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [total, setTotal] = useState(0);

  const load = useCallback(async (nextPage = 1, nextPageSize = 50) => {
    setLoading(true);
    setError(null);
    try {
      const data = await apiService.getAuditLogs(nextPage, nextPageSize);
      setLogs(data.items);
      setTotal(data.total);
      setPage(nextPage);
      setPageSize(nextPageSize);
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : '加载失败';
      setError(message);
      Message.error('获取审计日志失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const columns = [
    { title: '时间', dataIndex: 'created_at', width: 190, render: (value?: string) => value ? new Date(value).toLocaleString('zh-CN') : '—' },
    { title: '操作人', dataIndex: 'username', width: 110 },
    { title: '角色', dataIndex: 'role', width: 90, render: (value: string) => <Tag color="blue">{ROLE_LABEL[value] || value}</Tag> },
    { title: '方法', dataIndex: 'method', width: 90, render: (value: string) => <span className="cell-mono">{value}</span> },
    { title: '路径', dataIndex: 'path', render: (value: string) => <span className="cell-mono">{value}</span> },
    { title: '状态', dataIndex: 'status_code', width: 80, render: (value: number) => <Tag color={value < 400 ? 'green' : 'red'}>{value}</Tag> },
    { title: '来源 IP', dataIndex: 'source_ip', width: 140, render: (value?: string) => <span className="cell-mono">{value || '—'}</span> },
    { title: 'Request ID', dataIndex: 'request_id', width: 260, render: (value: string) => <span className="cell-mono">{value}</span> },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>审计日志</h1>
          <span className="sub">记录管理 API 的访问人、来源、路径和执行结果</span>
        </div>
      </div>
      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}
      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={logs}
          rowKey="id"
          loading={loading}
          pagination={{ current: page, pageSize, total, sizeCanChange: true, sizeOptions: [20, 50, 100, 200], onChange: (nextPage) => load(nextPage, pageSize), onPageSizeChange: (nextPageSize) => load(1, nextPageSize) }}
          scroll={{ x: 1200 }}
          noDataElement={<Empty description="暂无审计记录" />}
        />
      </Card>
    </div>
  );
}
