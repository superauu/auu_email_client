import { Component, type ErrorInfo, type ReactNode } from 'react';

export class ErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(_error: Error, _info: ErrorInfo) {
    // Never log message contents or credentials. Add opt-in sanitized diagnostics later.
  }
  render() {
    if (this.state.failed) return <main className="empty-state" role="alert"><h1>工作区暂时无法显示</h1><p>重新打开前，请先复制尚未保存的内容。</p><button className="button button-primary" onClick={() => location.reload()}>重新加载</button></main>;
    return this.props.children;
  }
}
