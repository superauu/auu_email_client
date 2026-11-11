import { create } from 'zustand';
import { desktopAccounts, type RealAccountInput } from '../platform/desktop';
import type { OAuthConfig, OAuthFlow } from '../domain/oauth';
export const useOAuthStore = create<{
  start: (account: RealAccountInput, config: OAuthConfig) => Promise<string>;
  read: (id: string) => Promise<OAuthFlow>;
  cancel: (id: string) => Promise<OAuthFlow>;
}>(() => ({ start: desktopAccounts.startOAuth, read: desktopAccounts.oauthStatus, cancel: desktopAccounts.cancelOAuth }));
