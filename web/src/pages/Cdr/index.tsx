import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Input,
  Select,
  DatePicker,
  Button,
  Space,
  Drawer,
  Descriptions,
  Message,
  Alert,
  Empty,
} from '@arco-design/web-react';
import { IconSearch, IconEye, IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { CdrEvent } from '@/types';
import StatusTag from '@/components/StatusTag';
import { extractSipUser } from '@/utils/sip';
import './Cdr.css';

const { RangePicker } = DatePicker;

function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  if (hours > 0) return `${hours}h ${minutes % 60}m ${seconds % 60}s`;
  if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
  return `${seconds}s`;
}

function formatDate(ms: number): string {
  return new Date(ms).toLocaleString('zh-CN');
}

export default function Cdr() {
  const [cdrs, setCdrs] = useState<CdrEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pagination, setPagination] = useState({ current: 1, pageSize: 20, total: 0 });
  const [filters, setFilters] = useState({
    caller: '',
    callee: '',
    status: '',
    dateRange: [] as string[],
  });
  const [drawerVisible, setDrawerVisible] = useState(false);
  const [selectedCdr, setSelectedCdr] = useState<CdrEvent | null>(null);
  const [hasRecording, setHasRecording] = useState(false);

  const loadCdrs = useCallback(
    async (page = 1, pageSize = pagination.pageSize) => {
      setLoading(true);
      setError(null);
      try {
        const params: any = { page, page_size: pageSize };
        if (filters.caller) params.caller = filters.caller;
        if (filters.callee) params.callee = filters.callee;
        if (filters.status) params.status = filters.status;
        if (filters.dateRange && filters.dateRange.length === 2) {
          params.start_time = new Date(filters.dateRange[0]).toISOString();
          params.end_time = new Date(filters.dateRange[1]).toISOString();
        }
        const result = await apiService.getCdrs(params);
        setCdrs(result.items);
        setPagination({ current: page, pageSize, total: result.total });
      } catch (err) {
        setError(err instanceof Error ? err.message : '加载失败');
        Message.error('获取呼叫记录失败');
      } finally {
        setLoading(false);
      }
    },
    [filters, pagination.pageSize]
  );

  useEffect(() => {
    loadCdrs(1, 20);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleSearch = () => loadCdrs(1, pagination.pageSize);

  const handleReset = () => {
    setFilters({ caller: '', callee: '', status: '', dateRange: [] });
    setTimeout(() => loadCdrs(1, pagination.pageSize), 0);
  };

  const handleViewDetail = async (record: CdrEvent) => {
    setSelectedCdr(record);
    setHasRecording(false);
    setDrawerVisible(true);
    try {
      const [cdr, recs] = await Promise.all([
        apiService.getCdr(record.call_id),
        apiService.getRecordings(),
      ]);
      if (cdr) setSelectedCdr(cdr);
      setHasRecording(!!recs.find((r) => r.call_id === record.call_id && r.has_audio));
    } catch {
      /* 使用列表中的记录 */
    }
  };

  const columns = [
    {
      title: '呼叫 ID',
      dataIndex: 'call_id',
      width: 200,
      ellipsis: true,
      render: (v: string) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '主叫',
      dataIndex: 'caller',
      width: 150,
      render: (c: string) => <span className="cell-mono">{extractSipUser(c)}</span>,
    },
    { title: '被叫', dataIndex: 'callee', width: 140 },
    {
      title: '开始时间',
      dataIndex: 'started_at_ms',
      width: 170,
      render: (ms: number) => <span className="cell-mono">{formatDate(ms)}</span>,
    },
    {
      title: '状态',
      dataIndex: 'status',
      width: 100,
      render: (s: string) => <StatusTag status={s} />,
    },
    {
      title: '通话时长',
      dataIndex: 'duration_ms',
      width: 110,
      render: (ms: number) => <span className="cell-mono">{formatDuration(ms)}</span>,
    },
    {
      title: '计费时长',
      dataIndex: 'billable_duration_ms',
      width: 110,
      render: (ms: number) => <span className="cell-mono">{formatDuration(ms)}</span>,
    },
    {
      title: 'MOS',
      dataIndex: 'mos',
      width: 80,
      render: (m: number) =>
        m ? <span className="cell-mono">{m.toFixed(1)}</span> : <span className="cell-dash">—</span>,
    },
    {
      title: '操作',
      dataIndex: 'actions',
      width: 90,
      fixed: 'right' as const,
      render: (_: any, record: CdrEvent) => (
        <Button type="text" size="small" icon={<IconEye />} onClick={() => handleViewDetail(record)}>
          详情
        </Button>
      ),
    },
  ];

  return (
    <div className="page-wrap cdr-page">
      <div className="page-header">
        <div className="page-header__title">
          <h1>呼叫记录</h1>
          <span className="sub">查询与分析所有呼叫详细记录（CDR）</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={() => loadCdrs(pagination.current, pagination.pageSize)}>
            刷新
          </Button>
        </div>
      </div>

      {error && (
        <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />
      )}

      <Card className="app-card filter-card" bordered={false}>
        <Space wrap size={[12, 12]}>
          <Input
            placeholder="主叫号码"
            style={{ width: 180 }}
            value={filters.caller}
            onChange={(v) => setFilters({ ...filters, caller: v })}
            allowClear
          />
          <Input
            placeholder="被叫号码"
            style={{ width: 180 }}
            value={filters.callee}
            onChange={(v) => setFilters({ ...filters, callee: v })}
            allowClear
          />
          <Select
            placeholder="呼叫状态"
            style={{ width: 140 }}
            value={filters.status || undefined}
            onChange={(v) => setFilters({ ...filters, status: v || '' })}
            allowClear
          >
            <Select.Option value="answered">已接通</Select.Option>
            <Select.Option value="canceled">已取消</Select.Option>
            <Select.Option value="failed">失败</Select.Option>
          </Select>
          <RangePicker
            showTime
            style={{ width: 340 }}
            value={filters.dateRange as any}
            onChange={(dates) => setFilters({ ...filters, dateRange: dates || [] })}
          />
          <Button type="primary" icon={<IconSearch />} onClick={handleSearch}>
            查询
          </Button>
          <Button onClick={handleReset}>重置</Button>
        </Space>
      </Card>

      <Card className="app-card" bordered={false} style={{ marginTop: 16 }}>
        <Table
          className="app-table"
          columns={columns}
          data={cdrs}
          rowKey="call_id"
          loading={loading}
          pagination={{
            current: pagination.current,
            pageSize: pagination.pageSize,
            total: pagination.total,
            sizeCanChange: true,
            showTotal: true,
            sizeOptions: [10, 20, 50, 100],
            onChange: (page, pageSize) => loadCdrs(page, pageSize),
          }}
          scroll={{ x: 1180 }}
          noDataElement={<Empty description="暂无呼叫记录" />}
        />
      </Card>

      <Drawer
        title="呼叫详情"
        width={680}
        visible={drawerVisible}
        onCancel={() => setDrawerVisible(false)}
        footer={null}
        className="cdr-drawer"
      >
        {selectedCdr && (
          <div className="cdr-detail">
            <div className="cdr-detail__hero">
              <div className="cdr-detail__hero-left">
                <div className="cdr-detail__from">{extractSipUser(selectedCdr.caller)}</div>
                <div className="cdr-detail__arrow">→</div>
                <div className="cdr-detail__to">{selectedCdr.callee || '—'}</div>
              </div>
              <StatusTag status={selectedCdr.status} />
            </div>

            <div className="cdr-detail__section">
              <div className="cdr-detail__section-title">基本信息</div>
              <Descriptions
                column={2}
                data={[
                  { label: '呼叫 ID', value: <span className="cell-mono">{selectedCdr.call_id}</span> },
                  { label: '开始时间', value: formatDate(selectedCdr.started_at_ms) },
                  { label: '应答时间', value: selectedCdr.answered_at_ms ? formatDate(selectedCdr.answered_at_ms) : '—' },
                  { label: '结束时间', value: formatDate(selectedCdr.ended_at_ms) },
                  { label: '通话时长', value: formatDuration(selectedCdr.duration_ms) },
                  { label: '计费时长', value: formatDuration(selectedCdr.billable_duration_ms) },
                ]}
              />
            </div>

            {selectedCdr.status === 'failed' && (
              <div className="cdr-detail__section">
                <div className="cdr-detail__section-title">失败信息</div>
                <Descriptions
                  column={2}
                  data={[
                    { label: 'SIP 状态码', value: <span className="cell-mono">{selectedCdr.failure_status_code || '—'}</span> },
                    { label: '失败原因', value: selectedCdr.failure_reason || '—' },
                  ]}
                />
              </div>
            )}

            <div className="cdr-detail__section">
              <div className="cdr-detail__section-title">媒体质量</div>
              <div className="quality-grid">
                <QualityItem label="MOS" value={selectedCdr.mos?.toFixed(2)} />
                <QualityItem label="主叫丢包率" value={selectedCdr.caller_rtcp_loss_rate != null ? `${selectedCdr.caller_rtcp_loss_rate.toFixed(3)}%` : undefined} />
                <QualityItem label="主叫抖动" value={selectedCdr.caller_rtcp_jitter_ms != null ? `${selectedCdr.caller_rtcp_jitter_ms.toFixed(2)}ms` : undefined} />
                <QualityItem label="主叫 RTT" value={selectedCdr.caller_rtcp_rtt_ms != null ? `${selectedCdr.caller_rtcp_rtt_ms}ms` : undefined} />
                <QualityItem label="网关丢包率" value={selectedCdr.gateway_rtcp_loss_rate != null ? `${selectedCdr.gateway_rtcp_loss_rate.toFixed(3)}%` : undefined} />
                <QualityItem label="网关抖动" value={selectedCdr.gateway_rtcp_jitter_ms != null ? `${selectedCdr.gateway_rtcp_jitter_ms.toFixed(2)}ms` : undefined} />
                <QualityItem label="网关 RTT" value={selectedCdr.gateway_rtcp_rtt_ms != null ? `${selectedCdr.gateway_rtcp_rtt_ms}ms` : undefined} />
              </div>
            </div>

            {selectedCdr.dtmf_digits && (
              <div className="cdr-detail__section">
                <div className="cdr-detail__section-title">DTMF 按键</div>
                <div className="dtmf-display">{selectedCdr.dtmf_digits}</div>
              </div>
            )}

            {hasRecording && (
              <div className="cdr-detail__section">
                <div className="cdr-detail__section-title">录音回放</div>
                <audio
                  controls
                  preload="none"
                  src={apiService.recordingAudioUrl(selectedCdr.call_id)}
                  style={{ width: '100%' }}
                />
              </div>
            )}
          </div>
        )}
      </Drawer>
    </div>
  );
}

function QualityItem({ label, value }: { label: string; value?: string }) {
  return (
    <div className="quality-item">
      <div className="quality-item__label">{label}</div>
      <div className={`quality-item__value font-num ${value ? '' : 'is-empty'}`}>
        {value || '—'}
      </div>
    </div>
  );
}
