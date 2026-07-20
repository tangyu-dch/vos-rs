# 前端优化记录

本文档整理 vos-rs Web 管理控制台的前端重构与样式优化内容，作为开发参考与后续维护基线。

## 1. 背景与目标

原前端基于 Argo Design + Tailwind v3，存在依赖老旧、样式不一致、文件臃肿（`console.tsx` 2082 行）、相对路径混乱等问题。本次优化目标：

1. 升级到 **HeroUI v2.8.0 + Tailwind v4**，统一组件库与设计语言
2. 按功能域拆分 `pages/` 子目录，降低单文件复杂度
3. 全局统一 `@/` 别名导入，消除相对路径噪音
4. 对齐 HeroUI 语义色规范，支持深色主题切换
5. 关键页面（仪表盘）对齐活跃通话页设计风格，提升视觉一致性

## 2. 技术栈升级

### 2.1 依赖版本

| 依赖 | 旧版本 | 新版本 | 说明 |
|------|--------|--------|------|
| `@heroui/react` | — | `^2.8.0` | HeroUI v2 组件库 |
| `tailwindcss` | `^3.x` | `^4.3.3` | Tailwind v4 CSS-first 配置 |
| `@tailwindcss/postcss` | — | `^4.3.3` | Tailwind v4 PostCSS 插件 |
| `sonner` | — | `^1.7.4` | Toast 通知组件 |
| `framer-motion` | — | `^12.42.2` | HeroUI 动画依赖 |

### 2.2 Tailwind v4 + HeroUI 集成要点

Tailwind v4 采用 CSS-first 配置，不再使用 `tailwind.config.js`。HeroUI v2 与 Tailwind v4 的正确集成方式：

1. **新建插件入口** `src/hero.ts`：

   ```ts
   import { heroui } from "@heroui/react";
   export default heroui();
   ```

2. **`src/index.css`** 通过 `@plugin` 引入插件，并用 `@source` 扫描 HeroUI theme dist：

   ```css
   @import "tailwindcss";
   @plugin './hero.ts';
   @source '../node_modules/@heroui/theme/dist/**/*.{js,ts,jsx,tsx}';
   @custom-variant dark (&:is(.dark *));
   ```

3. **PostCSS 配置** `postcss.config.js` 使用 default export：

   ```js
   import tailwindcss from '@tailwindcss/postcss';
   export default { plugins: [tailwindcss()] };
   ```

> **关键陷阱**：直接在 CSS 中使用 `@plugin "@heroui/theme"` 或 `@plugin "@heroui/theme/plugin"` 会触发 `k is not a function` 错误。必须通过独立的 `hero.ts` 文件调用 `heroui()` 并 default export。

### 2.3 主题切换机制

通过 `<html class="dark">` 切换深色模式，配合 `@custom-variant dark (&:is(.dark *))` 让 Tailwind v4 识别：

```tsx
// src/theme/ThemeContext.tsx
function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  root.classList.toggle('dark', theme === 'dark');
  root.style.colorScheme = theme;
  window.localStorage.setItem('vos-theme', theme);
}
```

Provider 注入顺序（`src/main.tsx`）：
`ThemeProvider → HeroUIProvider → BrowserRouter → AuthProvider`

## 3. 目录结构重构

### 3.1 拆分前

```
src/pages/
├── console.tsx           # 2082 行，所有页面 + 共享逻辑
├── trunk-detail.tsx      # 822 行
├── extension-detail.tsx
├── egress-group-detail.tsx
├── caller-pool-detail.tsx
├── agents.tsx
├── queues.tsx
├── ivr.tsx
└── Login/
```

### 3.2 拆分后

```
src/pages/
├── Login/                # 登录页
├── shared/               # 跨页面共享层
│   ├── index.ts          # 统一出口
│   ├── types.ts          # FieldSpec / ResourceSpec 类型
│   ├── format.ts         # callDetailText / valueText / moneyText 等
│   ├── resource-workspace.tsx  # 通用 CRUD 工作台
│   ├── resource-specs.ts       # 各资源的字段规格定义
│   ├── entity-detail.tsx       # 详情页通用骨架
│   └── call-detail.tsx         # 通话详情（SIP 流图、媒体诊断）
├── operations/           # 运营监控
│   ├── dashboard.tsx
│   ├── active-calls.tsx
│   └── call-detail.tsx
├── numbers/              # 号码管理
│   ├── extensions.tsx
│   ├── numbers.tsx
│   ├── did-destinations.tsx
│   ├── caller-pools.tsx
│   ├── extension-detail.tsx
│   └── caller-pool-detail.tsx
├── trunks/               # 中继管理
│   ├── access-trunks.tsx
│   ├── egress-trunks.tsx
│   ├── egress-groups.tsx
│   ├── trunk-detail.tsx
│   └── egress-group-detail.tsx
├── call-center/          # 呼叫中心
│   ├── agents.tsx
│   ├── queues.tsx
│   └── ivr.tsx
├── billing/              # 计费
│   ├── accounts.tsx
│   ├── rates.tsx
│   ├── transactions.tsx
│   └── calls.tsx
└── system/               # 系统配置
    ├── routes.tsx
    ├── security.tsx
    ├── infrastructure.tsx
    └── settings.tsx
```

### 3.3 拆分原则

1. **单一职责**：每个文件只承担一个明确职责
2. **公共接口不变**：通过 `shared/index.ts` 重新导出，外部 `use` 语句无需修改
3. **不修改业务逻辑**：纯重构行为，禁止"顺便改 bug"或"顺手优化算法"
4. **detail 文件迁移规则**：跨文件夹引用路径更新（如 `'./trunk-detail'` → `'../trunks/trunk-detail'`）

## 4. 导入路径统一

`vite.config.ts` 与 `tsconfig.json` 中已配置 `@/*` → `./src/*` 路径映射。全局将相对导入替换为 `@/` 别名：

| 原路径示例 | 新路径示例 |
|----------|----------|
| `./services/client` | `@/services/client` |
| `../components/detail-shell` | `@/components/detail-shell` |
| `../../services/trunks` | `@/services/trunks` |
| `../shared/format` | `@/pages/shared/format` |

**例外**：`src/main.tsx` 中的 `import './index.css'` 保留原样（Vite CSS 导入不能用别名）。

## 5. 视觉一致性优化

### 5.1 仪表盘页面对齐活跃通话页设计风格

**问题**：仪表盘页面与活跃通话页面存在视觉风格不一致：

| 维度 | 仪表盘（优化前） | 活跃通话（参考标准） |
|------|----------------|------------------|
| 标题栏 | 居中英雄横幅（`text-2xl`） | 左对齐（`text-base font-bold` + Chip + 副标题） |
| 刷新按钮 | `color="primary"` 实心按钮 | `variant="flat"` 描边按钮 |
| KPI 卡片 | `h-32`、`p-4`、`text-3xl` | — |
| 卡片内边距 | `p-6` | `p-4` |
| 整体留白 | 较多 | 紧凑 |
| 状态条颜色 | `bg-success-50 text-success-700`（Tailwind 默认色，深色主题失效） | HeroUI 语义色 `bg-success/10` |

**优化措施**：

1. **统一标题栏布局**：左对齐 `h2 text-base font-bold` + Chip（LIVE/10s 实时刷新）+ 副标题 + 右侧 `variant="flat"` 刷新按钮
2. **KPI 卡片紧凑化**：`gap-3`、`p-3`、数字 `text-2xl`
3. **统一卡片内边距**：`p-6` → `p-4`
4. **替换为 HeroUI 语义色**：`bg-success-50` → `bg-success/10`，`text-success-700` → `text-default-600` + Chip
5. **状态条用 Chip 替代纯文字**：路由引擎用 `<Chip color="success" variant="flat">LCR 就绪</Chip>`
6. **整体用单一大 Card 包裹**：与活跃通话页保持一致的"Card + 标题栏 + 内容"结构

### 5.2 HeroUI 语义色规范

所有颜色使用 HeroUI 语义色 token，避免使用 Tailwind 默认色（如 `bg-success-50`、`text-success-700`）：

| 类别 | HeroUI Token | 说明 |
|------|-------------|------|
| 前景文字 | `text-foreground` / `text-default-500` / `text-default-400` | 主文字 / 次要 / 辅助 |
| 背景 | `bg-content1` / `bg-content2` | 卡片背景 / 内嵌区域 |
| 主色 | `text-primary` / `bg-primary/10` | 主品牌色及透明度变体 |
| 成功 | `text-success` / `bg-success/10` | 成功状态 |
| 警告 | `text-warning` / `bg-warning/10` | 警告状态 |
| 危险 | `text-danger` / `bg-danger/10` / `border-danger/30` | 错误/危险状态 |
| 分隔线 | `border-divider` | 卡片内分隔 |
| 圆角 | `rounded-small` / `rounded-medium` / `rounded-large` | HeroUI 圆角 token |

### 5.3 按钮 variant 选择规范

| 场景 | variant | 示例 |
|------|---------|------|
| 主操作（提交/保存） | `solid`（默认） + `color="primary"` | 登录、保存配置 |
| 次要操作（刷新/取消） | `flat` | 刷新、取消 |
| 图标按钮（查看/编辑） | `light` + `isIconOnly` | 查看详情、编辑 |
| 危险操作（删除/挂断） | `flat` + `color="danger"` | 强拆挂断、删除 |
| 表格内操作 | `light` 或 `flat` + `size="sm"` | 表格行内按钮 |

## 6. 共享组件与工具

### 6.1 `components/detail-shell.tsx`

详情页通用骨架组件：

- `DetailHeader` — 详情页顶部（标题、刷新、保存按钮）
- `DetailLoading` — 加载中骨架
- `DetailErrorState` — 错误状态
- `FormGrid` — 表单网格布局
- `SectionBlock` — 区块容器（标题 + 描述 + 操作 + 内容）
- `ErrorState` — 列表页错误状态
- `LoadingState` — 列表页加载状态

### 6.2 `pages/shared/resource-workspace.tsx`

通用 CRUD 工作台，提供列表 + 表单 + 分页 + 搜索 + 状态筛选：

- `ResourceWorkspace` — 主组件，接收 `ResourceSpec`
- `usePageVisibility` — 页面可见性 Hook（用于自动刷新）
- `ConfirmDialog` — 通用确认弹窗
- `FormControl` — 根据 `FieldSpec` 渲染对应控件
- `resourceFormValues` / `resourceSaveValues` — 表单值转换工具

### 6.3 `utils/toast.ts`

基于 sonner 的 Toast 封装：

```ts
import { message } from '@/utils/toast';
message.success('保存成功');
message.error('保存失败');
```

## 7. 验证结果

| 验证项 | 命令 | 结果 |
|------|------|------|
| 类型检查 | `npx tsc --noEmit` | 0 错误 |
| 生产构建 | `npm run build` | 成功（3.68s） |
| 单元测试 | `npm test` | 42/42 通过 |
| ESLint | `npm run lint` | 0 warnings（1 个已知业务逻辑告警无关） |
| 浏览器实测 | 访问 `/overview`、`/calls/active`、`/extensions`、`/trunks/access`、`/login` | Input/Button/Card/Table 样式正常，控制台无错误 |

## 8. 提交记录

本次重构按功能拆分为 4 个提交：

1. `refactor(web): 升级 HeroUI v2.8.0 + Tailwind v4 并完成全局样式配置`
2. `refactor(web): pages 按功能域拆分并重构 ConsoleShell 与 Login`
3. `refactor(web): services/test/types 全局相对导入替换为 @ 别名`
4. `docs(web): 整理前端优化记录文档`（本文档）

## 9. 后续维护建议

1. **新增页面时**：按功能域放入对应子目录，引用共享层通过 `@/pages/shared/...`
2. **颜色使用**：禁止使用 Tailwind 默认色（`bg-green-50` 等），统一用 HeroUI 语义色
3. **按钮选择**：参考第 5.3 节的 variant 规范
4. **详情页开发**：优先复用 `shared/entity-detail.tsx` 的 `EntityDetail` 组件
5. **列表页开发**：优先使用 `shared/resource-workspace.tsx` 的 `ResourceWorkspace` + `resource-specs.ts` 中的字段规格
6. **主题适配**：所有自定义样式需同时验证浅色与深色主题
7. **单文件行数**：遵循 AGENTS.md 规范，业务逻辑文件不超过 300 行，超出应拆分
