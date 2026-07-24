import { useEffect, useRef, useState } from 'react';
import { Button, Chip, Progress, Tooltip } from '@heroui/react';
import { Upload, Play, Pause, Trash2, FileAudio, CheckCircle2, X } from 'lucide-react';
import { api } from '@/services/client';

interface PromptFile {
  filename: string;
  size: number;
  content_type: string;
  url: string;
}

interface AudioFileUploadProps {
  /** 当前已上传文件名 (后端返回的 stored_name) */
  value?: string;
  /** 文件变化回调, 传入后端返回的 filename 或空字符串 */
  onChange: (filename: string) => void;
  /** 字段标签 */
  label?: string;
  /** 提示文案 */
  hint?: string;
  /** 是否允许清空 (默认 true) */
  allowClear?: boolean;
}

/**
 * 音频文件上传组件
 *
 * - 点击选择文件 → 上传到 `/api/v1/ivr/prompts/upload`
 * - 上传成功后回显文件名 + 试听控件
 * - 支持清空已选文件
 * - 已上传文件可通过 `/api/v1/ivr/prompts/:filename` 试听
 */
export function AudioFileUpload({
  value,
  onChange,
  label = '音频文件',
  hint,
  allowClear = true,
}: AudioFileUploadProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const audioRef = useRef<HTMLAudioElement>(null);
  const [uploading, setUploading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [selectedFile, setSelectedFile] = useState<File | null>(null);

  // 当外部 value 变化时, 清空本地临时选择
  useEffect(() => {
    if (value) setSelectedFile(null);
  }, [value]);

  const audioUrl = value ? `/api/v1/ivr/prompts/${encodeURIComponent(value)}` : '';

  const handleFileSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setError(null);
    setSelectedFile(file);
    setUploading(true);
    setProgress(0);

    try {
      const formData = new FormData();
      formData.append('file', file);
      const result = await api.post<PromptFile>('/ivr/prompts/upload', formData, {
        headers: { 'Content-Type': 'multipart/form-data' },
        onUploadProgress: (evt) => {
          if (evt.total) setProgress(Math.round((evt.loaded / evt.total) * 100));
        },
      });
      onChange(result.filename);
      setProgress(100);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : '上传失败';
      setError(msg);
      setSelectedFile(null);
    } finally {
      setUploading(false);
      if (inputRef.current) inputRef.current.value = '';
    }
  };

  const handleClear = () => {
    onChange('');
    setSelectedFile(null);
    setError(null);
    setProgress(0);
    setPlaying(false);
    if (audioRef.current) {
      audioRef.current.pause();
      audioRef.current.currentTime = 0;
    }
  };

  const togglePlay = () => {
    if (!audioRef.current) return;
    if (playing) {
      audioRef.current.pause();
    } else {
      audioRef.current.play().catch(() => setError('试听失败, 文件可能已删除'));
    }
  };

  const displayName = value || selectedFile?.name || '';
  const fileSize = selectedFile?.size;

  return (
    <div className="flex flex-col gap-1.5">
      {label && (
        <label className="text-xs font-semibold text-foreground">{label}</label>
      )}

      <input
        ref={inputRef}
        type="file"
        accept="audio/wav,audio/mpeg,audio/mp3,audio/ogg,audio/gsm,.wav,.mp3,.ogg,.gsm"
        className="hidden"
        onChange={handleFileSelect}
      />

      {!displayName && !uploading && (
        <button
          type="button"
          onClick={() => inputRef.current?.click()}
          className="border-2 border-dashed border-default-200 hover:border-primary rounded-lg px-3 py-4 flex flex-col items-center gap-1.5 transition-colors text-default-400 hover:text-primary"
        >
          <Upload className="w-4 h-4" />
          <span className="text-xs">点击上传音频文件</span>
          <span className="text-[10px] text-default-400">支持 WAV / MP3 / GSM / OGG, 最大 50MB</span>
        </button>
      )}

      {uploading && (
        <div className="flex flex-col gap-1.5 px-3 py-2 border border-default-200 rounded-lg bg-content2">
          <div className="flex items-center gap-2 text-xs text-default-500">
            <FileAudio className="w-3.5 h-3.5 animate-pulse" />
            <span className="truncate">{selectedFile?.name}</span>
          </div>
          <Progress size="sm" value={progress} color="primary" aria-label="上传进度" />
        </div>
      )}

      {displayName && !uploading && (
        <div className="flex items-center gap-2 px-2.5 py-2 border border-default-200 rounded-lg bg-content2">
          {error ? (
            <X className="w-4 h-4 text-danger shrink-0" />
          ) : (
            <CheckCircle2 className="w-4 h-4 text-success shrink-0" />
          )}
          <div className="flex flex-col gap-0.5 min-w-0 flex-1">
            <span className="text-xs font-medium text-foreground truncate">{displayName}</span>
            {fileSize !== undefined && (
              <span className="text-[10px] text-default-400">
                {(fileSize / 1024).toFixed(1)} KB
              </span>
            )}
          </div>

          {audioUrl && !error && (
            <Tooltip content={playing ? '暂停' : '试听'} placement="top" delay={200}>
              <Button
                isIconOnly
                size="sm"
                variant="flat"
                color="primary"
                onPress={togglePlay}
                aria-label={playing ? '暂停试听' : '开始试听'}
              >
                {playing ? <Pause className="w-3.5 h-3.5" /> : <Play className="w-3.5 h-3.5" />}
              </Button>
            </Tooltip>
          )}

          <Tooltip content="重新上传" placement="top" delay={200}>
            <Button
              isIconOnly
              size="sm"
              variant="flat"
              onPress={() => inputRef.current?.click()}
              aria-label="重新上传"
            >
              <Upload className="w-3.5 h-3.5" />
            </Button>
          </Tooltip>

          {allowClear && (
            <Tooltip content="清空" placement="top" delay={200}>
              <Button
                isIconOnly
                size="sm"
                variant="flat"
                color="danger"
                onPress={handleClear}
                aria-label="清空文件"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </Button>
            </Tooltip>
          )}
        </div>
      )}

      {error && (
        <div className="flex items-center gap-1.5">
          <Chip size="sm" color="danger" variant="flat" className="text-[10px]">
            {error}
          </Chip>
        </div>
      )}

      {hint && !error && !displayName && (
        <span className="text-[10px] text-default-400">{hint}</span>
      )}

      {audioUrl && (
        <audio
          ref={audioRef}
          src={audioUrl}
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          onEnded={() => setPlaying(false)}
          onError={() => {
            setError('试听失败, 文件可能已删除');
            setPlaying(false);
          }}
          preload="none"
          className="hidden"
        />
      )}
    </div>
  );
}

export default AudioFileUpload;
