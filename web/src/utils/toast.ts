import { toast } from 'sonner';

/**
 * 全局消息提示工具，封装 sonner 以匹配原 Arco Message API。
 * 视觉与 HeroUI 主题保持一致（在 main.tsx 中通过 <Toaster theme={...} /> 注入主题）。
 */
export const message = {
  success(text: string) {
    toast.success(text);
  },
  error(text: string) {
    toast.error(text);
  },
  warning(text: string) {
    toast.warning(text);
  },
  info(text: string) {
    toast.info(text);
  },
};
