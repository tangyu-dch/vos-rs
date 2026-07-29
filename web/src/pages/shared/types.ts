// 资源工作台类型定义
// 从 console.tsx 拆分而来，供 ResourceWorkspace 及各资源页面共用

import type { Entity } from '@/services/resources';

export type FieldKind =
  'text' | 'textarea' | 'number' | 'duration' | 'switch' | 'select' | 'secret' | 'datetime';

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
  optionsResource?: 'egress-trunks' | 'allocation-source' | 'accounts' | 'tenants';
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

/** 服务端筛选字段配置 */
export interface ServerFilterSpec {
  /** 后端 query 参数名（同时作为筛选状态键） */
  param: string;
  /** 前端展示标签 */
  label: string;
  /** 筛选类型：keyword=关键字模糊搜索，status=状态下拉，select=固定选项，dateRange=时间范围 */
  kind: 'keyword' | 'status' | 'select' | 'dateRange';
  /** status/select 类型的选项列表（kind=keyword/dateRange 时忽略） */
  options?: Array<SelectOptionSpec>;
  /** 动态选项来源：复用表单字段选项加载机制（与 FieldSpec.optionsResource 取值一致） */
  optionsResource?: 'tenants' | 'accounts';
  /** keyword 类型的搜索框 placeholder */
  placeholder?: string;
  /**
   * 筛选模式：
   * - 'server'（默认）：作为 query 参数传给后端，由后端执行筛选
   * - 'client'：前端内存过滤（用于后端未支持筛选或无分页的端点）
   */
  mode?: 'server' | 'client';
  /**
   * 客户端筛选（mode='client'）时匹配的字段列表。
   * 默认 [param]。keyword 类型默认匹配行内所有字段。
   */
  clientFields?: string[];
  /**
   * dateRange 类型：开始时间对应的后端 query 参数名。
   * 筛选状态键为 `${param}_start`，传给后端时映射到此参数名。
   * 默认 `${param}_start`。
   */
  startParam?: string;
  /**
   * dateRange 类型：结束时间对应的后端 query 参数名。
   * 筛选状态键为 `${param}_end`，传给后端时映射到此参数名。
   * 默认 `${param}_end`。
   */
  endParam?: string;
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
  /** 服务端筛选配置：定义该资源后端支持的查询参数 */
  serverFilters?: ServerFilterSpec[];
  /** 自定义行操作按钮 (在操作列最左侧渲染) */
  customRowAction?: {
    label: string;
    icon?: string;
    color?: 'primary' | 'secondary' | 'success' | 'warning' | 'danger';
    onPress: (row: Entity) => void;
  };
}
