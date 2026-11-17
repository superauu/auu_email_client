import { Component, type ErrorInfo, type ReactNode } from 'react';
import { t } from '../../i18n/core';

export class ErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(_error: Error, _info: ErrorInfo) {
    // Never log message contents or credentials. Add opt-in sanitized diagnostics later.
  }
  render() {
    if (this.state.failed) return <main className="empty-state" role="alert"><h1>{t('ui.errorBoundaryTitle')}</h1><p>{t('ui.errorBoundaryBody')}</p><button className="button button-primary" onClick={() => location.reload()}>{t('app.reload')}</button></main>;
    return this.props.children;
  }
}
