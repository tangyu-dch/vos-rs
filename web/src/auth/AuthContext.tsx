import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { login as createSession } from '@/services/resources';
import { clearSession, getSession, saveSession, type AuthSession } from '@/services/auth';
import { api } from '@/services/client';

interface AuthContextValue {
  session: AuthSession | null;
  login: (username: string, password: string) => Promise<AuthSession>;
  logout: () => void;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<AuthSession | null>(() => getSession());

  // 其他标签页退出或切换账号时，当前标签页也必须立即失效，避免继续展示旧权限。
  useEffect(() => {
    const syncSession = (event: StorageEvent) => {
      if (event.key === 'vos-auth-session') {
        setSession(getSession());
      }
    };
    window.addEventListener('storage', syncSession);
    return () => window.removeEventListener('storage', syncSession);
  }, []);

  useEffect(() => {
    if (!session) return;
    const refreshProfile = async () => {
      try {
        const profile = await api.get<Omit<AuthSession, 'token'>>('/auth/me');
        const next = { ...profile, token: session.token };
        saveSession(next);
        setSession(next);
      } catch {
        // 请求客户端会统一处理失效令牌；临时网络异常时保留当前页面状态。
      }
    };
    void refreshProfile();
    const timer = window.setInterval(() => void refreshProfile(), 60_000);
    return () => window.clearInterval(timer);
  }, [session?.token]);

  const value = useMemo<AuthContextValue>(
    () => ({
      session,
      async login(username, password) {
        const next = await createSession(username, password);
        setSession(next);
        return next;
      },
      logout() {
        clearSession();
        setSession(null);
      },
    }),
    [session],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const value = useContext(AuthContext);
  if (!value) throw new Error('useAuth must be used inside AuthProvider');
  return value;
}
