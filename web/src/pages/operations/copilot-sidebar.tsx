//! Copilot 会话列表侧栏
//!
//! 展示当前操作员的 Copilot 历史会话，支持：
//! - 新建会话
//! - 切换会话（点击）
//! - 删除会话（hover 显示删除按钮）
//! - 置顶/取消置顶
//! - 折叠/展开侧栏

import { Button, Chip, ScrollShadow, Spinner } from '@heroui/react';
import {
  Bot, MessageSquare, PanelLeftClose, PanelLeftOpen, Pin, Plus, Trash2,
} from 'lucide-react';
import { CopilotSession, timeAgo } from './copilot-shared';

export interface SessionSidebarProps {
  sessions: CopilotSession[];
  currentId: string | null;
  loading: boolean;
  collapsed: boolean;
  /** 小屏（< lg）下是否展开为浮层。lg+ 始终在文档流内展示。 */
  mobileOpen: boolean;
  onSelect: (id: string) => void;
  onCreate: () => void;
  onDelete: (id: string) => void;
  onTogglePin: (id: string, pinned: boolean) => void;
  onToggleCollapse: () => void;
}

export function SessionSidebar({
  sessions, currentId, loading, collapsed, mobileOpen,
  onSelect, onCreate, onDelete, onTogglePin, onToggleCollapse,
}: SessionSidebarProps) {
  // 小屏默认隐藏（hidden lg:flex）；mobileOpen 时以 absolute 浮层形式覆盖主区域，lg+ 回到 static。
  const responsiveVisibility = mobileOpen
    ? 'flex absolute inset-y-0 left-0 z-40 shadow-xl lg:static lg:z-auto lg:shadow-none'
    : 'hidden lg:flex';

  if (collapsed) {
    return (
      <aside className={`w-12 shrink-0 border-r border-default-200 bg-content1 ${responsiveVisibility} flex-col items-center py-3 gap-3`}>
        <Button
          isIconOnly
          size="sm"
          variant="light"
          onPress={onToggleCollapse}
          aria-label="展开会话列表"
        >
          <PanelLeftOpen className="w-4 h-4 text-default-500" />
        </Button>
        <Button
          isIconOnly
          size="sm"
          color="primary"
          variant="flat"
          onPress={onCreate}
          aria-label="新建对话"
        >
          <Plus className="w-4 h-4" />
        </Button>
      </aside>
    );
  }

  return (
    <aside className={`w-64 shrink-0 border-r border-default-200 bg-content1 ${responsiveVisibility} flex-col`}>
      {/* 顶部：标题 + 折叠按钮 */}
      <div className="flex items-center justify-between px-3 py-3 border-b border-default-200">
        <div className="flex items-center gap-2 text-sm font-semibold text-foreground">
          <Bot className="w-4 h-4 text-primary" />
          <span>会话历史</span>
        </div>
        <Button
          isIconOnly
          size="sm"
          variant="light"
          onPress={onToggleCollapse}
          aria-label="折叠会话列表"
        >
          <PanelLeftClose className="w-4 h-4 text-default-500" />
        </Button>
      </div>

      {/* 新建对话按钮 */}
      <div className="p-2">
        <Button
          fullWidth
          size="sm"
          color="primary"
          variant="solid"
          onPress={onCreate}
          startContent={<Plus className="w-4 h-4" />}
        >
          新建对话
        </Button>
      </div>

      {/* 会话列表 */}
      <ScrollShadow className="flex-1 overflow-y-auto px-2 pb-2">
        {loading && (
          <div className="flex items-center justify-center py-8">
            <Spinner size="sm" />
          </div>
        )}
        {!loading && sessions.length === 0 && (
          <div className="text-center text-xs text-default-400 py-8 px-3">
            暂无历史会话，点击"新建对话"开始
          </div>
        )}
        {!loading && sessions.map((s) => {
          const active = s.id === currentId;
          return (
            <div
              key={s.id}
              role="button"
              tabIndex={0}
              onClick={() => onSelect(s.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onSelect(s.id);
                }
              }}
              className={`group relative px-2.5 py-2 mb-1 rounded-lg cursor-pointer transition-colors ${
                active
                  ? 'bg-primary/15 border-l-2 border-primary pl-2 shadow-sm'
                  : 'hover:bg-default-100 border-l-2 border-transparent'
              }`}
            >
              {/* 标题行 */}
              <div className="flex items-start justify-between gap-1.5">
                <div className="flex-1 min-w-0">
                  <div className={`text-xs truncate font-medium ${active ? 'text-primary' : 'text-foreground'}`}>
                    {s.title || '新对话'}
                  </div>
                  <div className="flex items-center gap-1.5 mt-1 text-[10px] text-default-400">
                    <MessageSquare className="w-2.5 h-2.5" />
                    <span>{s.message_count} 条</span>
                    {s.last_message_at && (
                      <>
                        <span>·</span>
                        <span>{timeAgo(s.last_message_at)}</span>
                      </>
                    )}
                  </div>
                </div>
                {/* 置顶 + 删除按钮（hover 显示） */}
                <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onTogglePin(s.id, !s.pinned);
                    }}
                    className={`p-1 rounded hover:bg-default-200 transition-colors ${
                      s.pinned ? 'text-primary' : 'text-default-400'
                    }`}
                    aria-label={s.pinned ? '取消置顶' : '置顶'}
                  >
                    <Pin className="w-3 h-3" />
                  </button>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onDelete(s.id);
                    }}
                    className="p-1 rounded hover:bg-danger/10 text-default-400 hover:text-danger transition-colors"
                    aria-label="删除会话"
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              </div>
              {/* 置顶/归档标记 */}
              {(s.pinned || s.archived) && (
                <div className="flex items-center gap-1 mt-1">
                  {s.pinned && (
                    <Chip size="sm" color="primary" variant="flat" className="text-[9px] h-4">
                      置顶
                    </Chip>
                  )}
                  {s.archived && (
                    <Chip size="sm" variant="flat" className="text-[9px] h-4 text-default-400">
                      归档
                    </Chip>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </ScrollShadow>
    </aside>
  );
}
