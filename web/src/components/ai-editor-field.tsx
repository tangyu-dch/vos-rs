import { useEffect, useRef, useState } from 'react';
import type { AiEditor as AiEditorInstance } from 'aieditor';
import 'aieditor/dist/style.css';

interface AiEditorFieldProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

const ANNOUNCEMENT_TOOLBAR = [
  'undo',
  'redo',
  'brush',
  'eraser',
  '|',
  'heading',
  'font-family',
  'font-size',
  '|',
  'bold',
  'italic',
  'underline',
  'strike',
  'link',
  'subscript',
  'superscript',
  '|',
  'highlight',
  'font-color',
  'align',
  'line-height',
  '|',
  'bullet-list',
  'ordered-list',
  'indent-decrease',
  'indent-increase',
  '|',
  'image',
  'quote',
  'code-block',
  'table',
  '|',
  'source-code',
  'fullscreen',
];

/** 公告正文富文本编辑器，负责同步 React 表单状态与编辑器生命周期。 */
export function AiEditorField({
  value,
  onChange,
  placeholder = '请输入公告内容',
}: AiEditorFieldProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<AiEditorInstance | null>(null);
  const onChangeRef = useRef(onChange);
  const [characterCount, setCharacterCount] = useState(() => value.replace(/<[^>]*>/g, '').length);

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    if (!containerRef.current) return;
    let disposed = false;
    let observer: MutationObserver | null = null;
    const element = containerRef.current;
    void import('aieditor').then(({ AiEditor }) => {
      if (disposed) return;
      const editor = new AiEditor({
        element,
        content: value,
        placeholder,
        lang: 'zh',
        theme: document.documentElement.classList.contains('dark') ? 'dark' : 'light',
        toolbarSize: 'small',
        toolbarKeys: ANNOUNCEMENT_TOOLBAR,
        contentRetention: false,
        draggable: false,
        image: { allowBase64: false },
        onChange: (current) => {
          setCharacterCount(current.getText().length);
          onChangeRef.current(current.getHtml());
        },
      });
      editorRef.current = editor;
      observer = new MutationObserver(() => {
        editor.changeTheme(document.documentElement.classList.contains('dark') ? 'dark' : 'light');
      });
      observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
    });
    return () => {
      disposed = true;
      observer?.disconnect();
      editorRef.current?.destroy();
      editorRef.current = null;
    };
  }, [placeholder]);

  useEffect(() => {
    const editor = editorRef.current;
    if (editor && value !== editor.getHtml()) editor.setContent(value || '');
  }, [value]);

  return (
    <div className="announcement-ai-editor overflow-hidden rounded-lg border border-default-200 bg-content1">
      <div ref={containerRef} className="h-[232px]" />
      <div className="flex h-7 items-center justify-end border-t border-default-200 px-3 text-tiny text-default-400">
        由 AiEditor 提供支持，字符数：{characterCount}
      </div>
    </div>
  );
}
