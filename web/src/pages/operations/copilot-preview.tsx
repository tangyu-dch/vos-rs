//! Copilot 预览 Modal 组件
//!
//! 拆分自 copilot.tsx，包含：
//! - ImageLightbox：图片大图预览弹窗
//! - CsvPreviewModal：CSV/文本文件内容查看器（CSV 渲染为表格，其他渲染为 pre）
//!
//! 纯展示组件，open 状态和内容由主页面控制。

import {
  Modal, ModalBody, ModalContent, ModalHeader,
} from '@heroui/react';
import { FileText } from 'lucide-react';

// ============ 图片大图预览 ============

export interface ImageLightboxProps {
  url: string | null;
  onClose: () => void;
}

/** 图片大图预览 Lightbox 弹窗（url 为 null 时关闭） */
export function ImageLightbox({ url, onClose }: ImageLightboxProps) {
  return (
    <Modal
      isOpen={url !== null}
      onClose={onClose}
      size="4xl"
      scrollBehavior="inside"
      classNames={{
        backdrop: 'bg-black/80 backdrop-blur-md',
        base: 'bg-content1/95 border border-default-200 shadow-2xl rounded-2xl',
      }}
    >
      <ModalContent>
        <ModalHeader className="flex items-center justify-between text-sm font-bold border-b border-default-100">
          <span>图片大图预览</span>
        </ModalHeader>
        <ModalBody className="p-6 flex items-center justify-center min-h-[350px]">
          {url && (
            <img
              src={url}
              alt="大图预览"
              className="max-h-[75vh] max-w-full rounded-xl object-contain shadow-2xl border border-default-200"
            />
          )}
        </ModalBody>
      </ModalContent>
    </Modal>
  );
}

// ============ CSV / 文本文件预览 ============

export interface CsvPreviewModalProps {
  file: { name: string; content: string } | null;
  onClose: () => void;
}

/** CSV/文本文件内容查看器：CSV 渲染为表格，其他渲染为 pre 代码块 */
export function CsvPreviewModal({ file, onClose }: CsvPreviewModalProps) {
  const isCsv = file?.name.endsWith('.csv') ?? false;
  return (
    <Modal
      isOpen={file !== null}
      onClose={onClose}
      size="4xl"
      scrollBehavior="inside"
      classNames={{
        backdrop: 'bg-black/80 backdrop-blur-md',
        base: 'bg-content1/95 border border-default-200 shadow-2xl rounded-2xl max-h-[85vh]',
      }}
    >
      <ModalContent>
        <ModalHeader className="flex items-center justify-between text-sm font-bold border-b border-default-100">
          <div className="flex items-center gap-2">
            <FileText className="w-4 h-4 text-primary" />
            <span>文件内容预览: {file?.name}</span>
          </div>
        </ModalHeader>
        <ModalBody className="p-4 overflow-x-auto">
          {file && (
            isCsv ? (
              <CsvTable content={file.content} />
            ) : (
              <pre className="p-4 rounded-xl bg-default-100 text-xs font-mono whitespace-pre-wrap overflow-x-auto text-foreground border border-default-200">
                {file.content}
              </pre>
            )
          )}
        </ModalBody>
      </ModalContent>
    </Modal>
  );
}

/** CSV 内容渲染为表格（首行作为表头） */
function CsvTable({ content }: { content: string }) {
  const rows = content.split('\n');
  const header = rows[0]?.split(',').map((h) => h.trim().replace(/^"|"$/g, '')) ?? [];
  const bodyRows = rows.slice(1).filter((row) => row.trim().length > 0);

  return (
    <div className="w-full overflow-x-auto border border-default-200 rounded-xl">
      <table className="w-full text-xs text-left border-collapse">
        <thead>
          <tr className="bg-default-100 border-b border-default-200 text-foreground font-semibold">
            {header.map((h, i) => (
              <th key={i} className="px-3 py-2 border-r border-default-200 last:border-r-0 whitespace-nowrap">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {bodyRows.map((row, rIdx) => (
            <tr key={rIdx} className="border-b border-default-100 hover:bg-default-50/50">
              {row.split(',').map((cell, cIdx) => (
                <td key={cIdx} className="px-3 py-1.5 border-r border-default-100 last:border-r-0 whitespace-nowrap text-default-600">
                  {cell.trim().replace(/^"|"$/g, '')}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
