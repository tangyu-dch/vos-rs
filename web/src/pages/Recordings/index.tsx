import { useState, useCallback, useRef } from 'react';
import {
  Card,
  Table,
  Button,
  Alert,
  Message,
  Empty,
  Tag,
} from '@arco-design/web-react';
import { IconRefresh, IconDownload, IconPlayArrow, IconPause } from '@arco-design/web-react/icon';
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
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [playingId, setPlayingId] = useState<string | null>(null);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [loadingAudio, setLoadingAudio] = useState(false);
  const audioRef = useRef<HTMLAudioElement>(null);

  const load = useCallback(async (nextPage = 1, nextPageSize = 20) => {
    setLoading(true);
    setError(null);
    try {
      const result = await apiService.getRecordings(nextPage, nextPageSize);
      setData(result.items);
      setTotal(result.total);
      setPage(nextPage);
      setPageSize(nextPageSize);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载失败');
      Message.error('获取录音列表失败');
    } finally {
      setLoading(false);
    }
  }, []);

  const playRecording = async (callId: string) => {
    // If already playing this recording, pause it
    if (playingId === callId && audioRef.current) {
      if (audioRef.current.paused) {
        audioRef.current.play();
      } else {
        audioRef.current.pause();
      }
      return;
    }

    // Stop current playback and release URL
    if (audioUrl) {
      URL.revokeObjectURL(audioUrl);
    }

    // Load new recording
    setLoadingAudio(true);
    try {
      const blob = await apiService.getRecordingAudio(callId);
      const url = URL.createObjectURL(blob);
      setAudioUrl(url);
      setPlayingId(callId);
    } catch {
      Message.error('加载录音失败');
      setPlayingId(null);
      setAudioUrl(null);
    } finally {
      setLoadingAudio(false);
    }
  };

  const handleAudioEnded = () => {
    if (audioUrl) {
      URL.revokeObjectURL(audioUrl);
    }
    setPlayingId(null);
    setAudioUrl(null);
  };

  const downloadRecording = async (callId: string) => {
    try {
      const blob = await apiService.getRecordingAudio(callId);
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = `${callId}.wav`;
      link.click();
      URL.revokeObjectURL(url);
    } catch {
      Message.error('下载录音失败');
    }
  };

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
      title: '时长',
      dataIndex: 'duration_secs',
      width: 80,
      render: (v: number) => v > 0 ? `${v.toFixed(1)}s` : '—',
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
      render: (callId: string, record: RecordingInfo) => {
        if (!record.has_audio) return '—';
        const isPlaying = playingId === callId;
        return (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Button
              type="text"
              size="small"
              icon={isPlaying && !audioRef.current?.paused ? <IconPause /> : <IconPlayArrow />}
              loading={loadingAudio && playingId === callId}
              onClick={() => playRecording(callId)}
            />
            {isPlaying && audioUrl && (
              <audio
                ref={audioRef}
                controls
                src={audioUrl}
                onEnded={handleAudioEnded}
                style={{ height: 32, verticalAlign: 'middle', flex: 1 }}
              />
            )}
          </div>
        );
      },
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
            onClick={() => downloadRecording(record.call_id)}
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
          <Button icon={<IconRefresh />} onClick={() => load(page, pageSize)}>
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
          pagination={{ current: page, pageSize, total, sizeCanChange: true, sizeOptions: [10, 20, 50, 100], onChange: (nextPage) => load(nextPage, pageSize), onPageSizeChange: (nextPageSize) => load(1, nextPageSize) }}
          scroll={{ x: 1000 }}
          noDataElement={<Empty description="暂无录音（接通呼叫结束后生成）" />}
        />
      </Card>
    </div>
  );
}
