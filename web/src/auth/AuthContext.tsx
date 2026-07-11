import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { apiService } from '@/services/api';
import { clearSession, getSession, type AuthSession } from '@/services/auth';

interface AuthContextValue {
  session: AuthSession | null;
  login: (username: string, password: string) => Promise<void>;
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

  const value = useMemo<AuthContextValue>(
    () => ({
      session,
      async login(username, password) {
        const next = await apiService.login(username, password);
        setSession(next);
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
