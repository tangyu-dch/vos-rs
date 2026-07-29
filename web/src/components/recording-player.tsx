import { useEffect, useState } from 'react';
import { Spinner } from '@heroui/react';
import { api, ApiError } from '@/services/client';

interface RecordingPlayerProps {
  source: string;
  className?: string;
}

export function RecordingPlayer({ source, className = 'w-full h-8' }: RecordingPlayerProps) {
  const [audioUrl, setAudioUrl] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let disposed = false;
    let objectUrl = '';
    setAudioUrl('');
    setError('');
    setLoading(true);

    void api
      .blob(source)
      .then((blob) => {
        if (disposed) return;
        if (blob.size === 0) {
          setError('本通话未收到可录制的音频媒体，未生成录音文件');
          return;
        }
        objectUrl = URL.createObjectURL(blob);
        setAudioUrl(objectUrl);
      })
      .catch((requestError: unknown) => {
        if (!disposed) {
          if (requestError instanceof ApiError && requestError.status === 404) {
            setError('本通话未收到可录制的音频媒体，未生成录音文件');
          } else if (requestError instanceof ApiError && requestError.status === 503) {
            setError('录音存储配置不可用，请检查存储连接');
          } else {
            setError(requestError instanceof Error ? requestError.message : '录音加载失败');
          }
        }
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });

    return () => {
      disposed = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [source]);

  if (loading) {
    return <Spinner size="sm" color="primary" label="正在加载录音" />;
  }
  if (error) {
    return <p className="text-small text-danger">{error}</p>;
  }
  return (
    <audio
      controls
      className={className}
      src={audioUrl}
      onError={() => setError('录音格式损坏或暂不可播放')}
      onLoadedMetadata={(event) => {
        if (!Number.isFinite(event.currentTarget.duration) || event.currentTarget.duration <= 0) {
          setError('录音文件中没有可播放的音频数据');
        }
      }}
    />
  );
}
