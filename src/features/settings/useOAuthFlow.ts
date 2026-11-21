import { useEffect, useRef, useState } from 'react';
import { useOAuthStore } from '../../stores/oauthStore';
import type { OAuthConfig, OAuthFlow } from '../../domain/oauth';
import type { RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';
import { t } from '../../i18n/core';

/** OAuth 阶段文案（每次调用取当前语言，避免模块加载期固化）。 */
export function oauthPhaseText(state: OAuthFlow['state']): string {
  const phases: Record<OAuthFlow['state'], Parameters<typeof t>[0]> = {
    waiting: 'oauth.phaseWaiting', exchanging: 'oauth.phaseExchanging', verifying: 'oauth.phaseVerifying',
    committing: 'oauth.phaseCommitting', saved: 'oauth.phaseSaved', failed: 'oauth.phaseFailed', cancelled: 'oauth.phaseCancelled',
  };
  return t(phases[state]);
}

/** 共享的授权会话：发起、轮询、取消与完成后的收尾，供添加向导与重新授权页复用。 */
export function useOAuthFlow() {
  const [flow, setFlow] = useState<OAuthFlow | null>(null);
  const [flowId, setFlowId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const saved = useRef(false);
  const running = busy || !!flowId && !flow || !!flow && !['saved', 'failed', 'cancelled'].includes(flow.state);
  useEffect(() => {
    if (!flowId) return;
    let active = true; let timer: ReturnType<typeof setTimeout> | undefined;
    async function poll() {
      try {
        const value = await useOAuthStore.getState().read(flowId!);
        if (!active) return;
        setFlow(value);
        if (['saved', 'failed', 'cancelled'].includes(value.state)) return;
      } catch (pollError) { if (active) setError(errorMessage(pollError)); }
      if (active) timer = setTimeout(() => void poll(), 1000);
    }
    void poll();
    return () => {
      active = false; clearTimeout(timer);
      // 卸载时必须读取 ref 的最新值：只有账号尚未保存成功才取消授权会话。
      // eslint-disable-next-line react-hooks/exhaustive-deps -- 此处读最新 ref 值正是意图
      if (!saved.current) void useOAuthStore.getState().cancel(flowId).catch(() => {});
    };
  }, [flowId]);
  async function begin(account: RealAccountInput, config: OAuthConfig) {
    setBusy(true); setError(''); setFlow(null);
    try { setFlowId(await useOAuthStore.getState().start(account, config)); }
    catch (startError) { setError(errorMessage(startError)); }
    setBusy(false);
  }
  async function stop(): Promise<OAuthFlow | null> {
    if (!flowId) return null;
    try { const value = await useOAuthStore.getState().cancel(flowId); setFlow(value); return value; }
    catch (cancelError) { setError(errorMessage(cancelError)); return null; }
  }
  return { flow, error, setError, running, begin, stop, savedRef: saved };
}
