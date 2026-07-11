import { useEffect, useState } from 'react';

/** 返回当前浏览器标签页是否可见，用于暂停后台轮询。 */
export function usePageVisibility(): boolean {
  const [visible, setVisible] = useState(() => document.visibilityState !== 'hidden');

  useEffect(() => {
    const handleVisibilityChange = () => {
      setVisible(document.visibilityState !== 'hidden');
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => document.removeEventListener('visibilitychange', handleVisibilityChange);
  }, []);

  return visible;
}
