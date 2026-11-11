export interface OAuthConfig { provider: 'google' | 'microsoft'; clientId: string; tenant: string }
export interface OAuthFlow { id: string; state: 'waiting' | 'exchanging' | 'verifying' | 'committing' | 'saved' | 'failed' | 'cancelled'; accountId: string | null; error: string | null; folderCount: number | null }
