import { useState, useEffect, useCallback } from 'react';
import {
  Card,
  Table,
  Button,
  Alert,
  Message,
  Empty,
  Tag,
} from '@arco-design/web-react';
import { IconRefresh, IconDownload } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { RecordingInfo } from '@/types';

function formatSize(bytes: number): string {
  if (!bytes) return '—';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

export default function Recordings() {
  const [data, setData] = useState<RecordingInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setData(await apiService.getRecordings());
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取录音列表失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const columns = [
    {
      title: 'Call ID',
      dataIndex: 'call_id',
      render: (v: string) => <span className="cell-mono">{v}</span>,
    },
    {
      title: '创建时间',
      dataIndex: 'created_at_ms',
      width: 180,
      render: (ms: number) => new Date(ms).toLocaleString('zh-CN'),
    },
    {
      title: '文件大小',
      dataIndex: 'size_bytes',
      width: 110,
      render: formatSize,
    },
    {
      title: '状态',
      dataIndex: 'has_audio',
      width: 90,
      render: (has: boolean) =>
        has ? <Tag color="green">可播放</Tag> : <Tag color="gray">无音频</Tag>,
    },
    {
      title: '在线试听',
      dataIndex: 'call_id',
      width: 340,
      render: (callId: string, record: RecordingInfo) =>
        record.has_audio ? (
          <audio
            controls
            preload="none"
            src={apiService.recordingAudioUrl(callId)}
            style={{ height: 32, verticalAlign: 'middle' }}
          />
        ) : (
          '—'
        ),
    },
    {
      title: '操作',
      dataIndex: 'actions',
      width: 100,
      fixed: 'right' as const,
      render: (_: any, record: RecordingInfo) =>
        record.has_audio ? (
          <Button
            type="text"
            size="small"
            icon={<IconDownload />}
            onClick={() => window.open(apiService.recordingAudioUrl(record.call_id))}
          >
            下载
          </Button>
        ) : (
          '—'
        ),
    },
  ];

  return (
    <div className="page-wrap">
      <div className="page-header">
        <div className="page-header__title">
          <h1>录音</h1>
          <span className="sub">呼叫录音在线试听与下载（G.711 双声道 8kHz/16bit WAV）</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={load}>
            刷新
          </Button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      <Card className="app-card" bordered={false}>
        <Table
          className="app-table"
          columns={columns}
          data={data}
          rowKey="call_id"
          loading={loading}
          pagination={{ pageSize: 20, sizeCanChange: true }}
          scroll={{ x: 1000 }}
          noDataElement={<Empty description="暂无录音（接通呼叫结束后生成）" />}
        />
      </Card>
    </div>
  );
}
