import { createContext, useContext, useMemo, useState, type ReactNode } from 'react';
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
