import { useEffect, useRef, useState } from 'react';
import { useOAuthStore } from '../../stores/oauthStore';
import type { OAuthConfig, OAuthFlow } from '../../domain/oauth';
import type { RealAccountInput } from '../../platform/desktop';
import { errorMessage } from '../../domain/errors';

export const oauthPhaseText: Record<OAuthFlow['state'], string> = {
  waiting: '等待系统浏览器授权…',
  exchanging: '正在取得授权…',
  verifying: '正在验证 IMAP 登录和文件夹…',
  committing: '正在安全保存账号…',
  saved: '授权和 IMAP 验证通过，账号已保存。',
  failed: '授权未完成。',
  cancelled: '授权已取消，账号未保存。',
};

/** 共享的授权会话：发起、轮询、取消与完成后的收尾，供向导内嵌进度与重新授权对话框复用。 */
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
