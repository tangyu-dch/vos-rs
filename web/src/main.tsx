import React from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import { HeroUIProvider } from '@heroui/react';
import { Toaster } from 'sonner';
import App from '@/App';
import { AuthProvider } from '@/auth/AuthContext';
import { ThemeProvider, useTheme } from '@/theme/ThemeContext';
import './index.css';

function ToasterWithTheme() {
  const { theme } = useTheme();
  return <Toaster theme={theme} position="top-right" richColors closeButton />;
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ThemeProvider>
      <HeroUIProvider>
        <BrowserRouter>
          <AuthProvider>
            <App />
            <ToasterWithTheme />
          </AuthProvider>
        </BrowserRouter>
      </HeroUIProvider>
    </ThemeProvider>
  </React.StrictMode>,
);
