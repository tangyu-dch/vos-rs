//! Copilot 底部输入框组件
//!
//! 拆分自 copilot.tsx，包含：
//! - AttachedFile 类型定义（附件内部表示，供主页面和本组件共享）
//! - AttachmentChips：已附件列表预览（Chip Pills，可点击预览/删除）
//! - ComposerBar：底部浮动输入框（上传按钮 + 输入框 + 发送按钮）
//!
//! 纯展示组件，状态由主页面管理，通过 props 传入。

import { RefObject } from 'react';
import { Button, Input } from '@heroui/react';
import {
  Image as ImageIcon, Paperclip, Send, X, FileText,
} from 'lucide-react';

// ============ 附件类型定义 ============

/** 输入框附件内部表示（与 MessageItem.files 不同，含 File 对象和预览数据） */
export interface AttachedFile {
  id: string;
  file: File;
  name: string;
  sizeStr: string;
  isImage: boolean;
  previewUrl?: string;
  textContent?: string;
  base64Data?: string;
}

// ============ 附件预览 Chips ============

export interface AttachmentChipsProps {
  files: AttachedFile[];
  onRemove: (id: string) => void;
  onPreviewImage: (url: string) => void;
}

/** 已附件列表预览：Chip Pills 形式，点击图片可预览，点 X 删除 */
export function AttachmentChips({ files, onRemove, onPreviewImage }: AttachmentChipsProps) {
  if (files.length === 0) return null;
  return (
    <div className="w-full max-w-full lg:max-w-[94%] mx-auto flex flex-wrap items-center gap-2 px-2">
      {files.map((file) => (
        <div
          key={file.id}
          className="flex items-center gap-1.5 px-2.5 py-1 bg-default-100 dark:bg-default-50/10 border border-default-200 rounded-full text-xs text-foreground shadow-sm transition-all cursor-pointer hover:border-primary/50"
          onClick={() => file.previewUrl && onPreviewImage(file.previewUrl)}
        >
          {file.isImage ? (
            file.previewUrl ? (
              <img src={file.previewUrl} alt={file.name} className="w-4 h-4 rounded object-cover" />
            ) : (
              <ImageIcon className="w-3.5 h-3.5 text-primary" />
            )
          ) : (
            <FileText className="w-3.5 h-3.5 text-secondary" />
          )}
          <span className="max-w-[140px] truncate font-medium">{file.name}</span>
          <span className="text-[10px] text-default-400">({file.sizeStr})</span>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onRemove(file.id);
            }}
            className="hover:text-danger p-0.5 rounded-full transition-colors ml-0.5"
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      ))}
    </div>
  );
}

// ============ 底部输入框 ============

export interface ComposerBarProps {
  inputQuery: string;
  setInputQuery: (v: string) => void;
  sending: boolean;
  onSend: () => void;
  onPaste: (e: React.ClipboardEvent) => void;
  fileInputRef: RefObject<HTMLInputElement>;
  onFileSelect: (files: FileList) => void;
}

/** 底部浮动输入框：隐藏 FileInput + 上传按钮 + 文本输入 + 发送按钮 */
export function ComposerBar({
  inputQuery, setInputQuery, sending, onSend, onPaste, fileInputRef, onFileSelect,
}: ComposerBarProps) {
  return (
    <div className="w-full px-4 py-4 shrink-0 bg-transparent flex flex-col gap-2">
      {/* 隐藏的 File Input 用于激活本地文件选择 */}
      <input
        type="file"
        ref={fileInputRef}
        className="hidden"
        multiple
        accept=".csv,.json,.log,.txt,image/*"
        onChange={(e) => {
          if (e.target.files && e.target.files.length > 0) {
            onFileSelect(e.target.files);
            e.target.value = '';
          }
        }}
      />

      <div className="w-full max-w-full lg:max-w-[94%] mx-auto rounded-3xl border-2 border-default-200 hover:border-primary/40 focus-within:border-primary focus-within:ring-4 focus-within:ring-primary/10 bg-content1 shadow-lg p-2 flex items-center gap-2 transition-all duration-200">
        <Input
          variant="flat"
          classNames={{
            inputWrapper: 'bg-transparent shadow-none hover:bg-transparent focus-within:bg-transparent',
            input: 'text-sm',
          }}
          placeholder="询问问题、粘贴图片/日志、或上传 CSV 进行导入排查... (可直接 Ctrl+V / Cmd+V 粘贴截图)"
          value={inputQuery}
          onValueChange={setInputQuery}
          onKeyDown={(e) => e.key === 'Enter' && !e.shiftKey && (e.preventDefault(), onSend())}
          onPaste={onPaste}
          isDisabled={sending}
          startContent={
            <div className="flex items-center gap-1.5 text-default-400 mr-1">
              <button
                type="button"
                title="上传数据/日志文件 (.csv, .json, .txt, .log)"
                onClick={() => fileInputRef.current?.click()}
                className="p-1 hover:text-primary hover:bg-default-100 rounded-lg transition-colors"
              >
                <Paperclip className="w-4 h-4" />
              </button>
              <button
                type="button"
                title="截图/图片识别异常"
                onClick={() => fileInputRef.current?.click()}
                className="p-1 hover:text-primary hover:bg-default-100 rounded-lg transition-colors"
              >
                <ImageIcon className="w-4 h-4" />
              </button>
            </div>
          }
          endContent={
            <Button
              size="sm"
              color="primary"
              className="rounded-2xl px-4 text-primary-foreground font-bold"
              isLoading={sending}
              onPress={onSend}
              startContent={!sending && <Send className="w-3.5 h-3.5" />}
            >
              发送
            </Button>
          }
        />
      </div>
    </div>
  );
}
