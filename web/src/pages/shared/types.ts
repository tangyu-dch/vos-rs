// 资源工作台类型定义
// 从 console.tsx 拆分而来，供 ResourceWorkspace 及各资源页面共用

export type FieldKind = 'text' | 'textarea' | 'number' | 'duration' | 'switch' | 'select' | 'secret';

export interface SelectOptionSpec {
  label: string;
  value: string;
}

export interface FieldSpec {
  key: string;
  label: string;
  kind?: FieldKind;
  required?: boolean;
  options?: Array<string | SelectOptionSpec>;
  optionsResource?: 'egress-trunks' | 'allocation-source';
  readonly?: boolean;
  defaultValue?: unknown;
  fullWidth?: boolean;
  min?: number;
  placeholder?: string;
  pattern?: RegExp;
  patternMessage?: string;
  preserveEmptyOnEdit?: boolean;
  showWhen?: (draft: Record<string, unknown>) => boolean;
}

export interface ResourceSpec {
  title: string;
  description: string;
  path: string;
  params?: Record<string, string>;
  idKey: string;
  fields: FieldSpec[];
  detailPath?: string;
  createLabel?: string;
  readOnly?: boolean;
  action?: 'credit';
}
