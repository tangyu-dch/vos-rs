import { Component, type ErrorInfo, type ReactNode } from 'react';

interface AppErrorBoundaryProps {
  children: ReactNode;
}

interface AppErrorBoundaryState {
  hasError: boolean;
}

/** 捕获页面渲染异常，避免单个懒加载页面导致整个管理后台白屏。 */
export default class AppErrorBoundary extends Component<AppErrorBoundaryProps, AppErrorBoundaryState> {
  state: AppErrorBoundaryState = { hasError: false };

  static getDerivedStateFromError(): AppErrorBoundaryState {
    return { hasError: true };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    // 保留浏览器控制台诊断信息；生产环境可在这里接入统一错误上报服务。
    console.error('页面渲染异常', error, errorInfo);
  }

  private handleReload = (): void => {
    window.location.reload();
  };

  render() {
    if (!this.state.hasError) return this.props.children;

    return (
      <main className="app-error-boundary" role="alert">
        <div className="app-error-boundary__code">500</div>
        <h1>页面加载异常</h1>
        <p>当前页面遇到暂时性问题，请刷新后重试。</p>
        <div className="app-error-boundary__actions">
          <button type="button" onClick={this.handleReload}>刷新页面</button>
          <a href="/dashboard">返回工作台</a>
        </div>
      </main>
    );
  }
}
