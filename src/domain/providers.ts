import { t } from '../i18n/core';
// 提供商注册表：新增平台只需追加一条记录。
// available=false 的卡片显示为禁用「即将支持」，不伪装可用能力。
export type ProviderId = 'gmail' | 'microsoft' | 'qq' | '163' | 'pop3' | 'imap';

export interface ProviderEndpoints {
  imapHost: string;
  imapPort: number;
  imapSecurity: 'tls' | 'starttls';
  smtpHost: string;
  smtpPort: number;
}

export interface ProviderPreset {
  id: ProviderId;
  /** 显示名的消息键（providerPresetLabel 解析为当前语言）。 */
  labelKey: Parameters<typeof t>[0];
  descriptionKey: Parameters<typeof t>[0];
  available: boolean;
  /** OAuth 走 mail_oauth_start；密码 IMAP 走账号向导保存路径。 */
  oauth: boolean;
  endpoints?: ProviderEndpoints;
  /** 密码提示的消息键；解析见 providerPresetAuthHint。 */
  authHintKey?: Parameters<typeof t>[0];
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'gmail',
    labelKey: 'provider.imap.label', // 占位；Gmail 品牌名在 providerPresetLabel 特判
    descriptionKey: 'provider.gmail.desc',
    available: true,
    oauth: true,
  },
  {
    id: 'microsoft',
    labelKey: 'provider.microsoft.label',
    descriptionKey: 'provider.microsoft.desc',
    available: false,
    oauth: true,
  },
  {
    id: 'qq',
    labelKey: 'provider.qq.label',
    descriptionKey: 'provider.qq.desc',
    available: true,
    oauth: false,
    endpoints: { imapHost: 'imap.qq.com', imapPort: 993, imapSecurity: 'tls', smtpHost: 'smtp.qq.com', smtpPort: 465 },
    authHintKey: 'provider.qq.authHint',
  },
  {
    id: '163',
    labelKey: 'provider.163.label',
    descriptionKey: 'provider.163.desc',
    available: true,
    oauth: false,
    endpoints: { imapHost: 'imap.163.com', imapPort: 993, imapSecurity: 'tls', smtpHost: 'smtp.163.com', smtpPort: 465 },
    authHintKey: 'provider.163.authHint',
  },
  {
    id: 'pop3',
    labelKey: 'provider.pop3.label',
    descriptionKey: 'provider.pop3.desc',
    available: true,
    oauth: false,
  },
  {
    id: 'imap',
    labelKey: 'provider.imap.label',
    descriptionKey: 'provider.imap.desc',
    available: true,
    oauth: false,
  },
];

export function providerPreset(id: ProviderId): ProviderPreset {
  const preset = PROVIDER_PRESETS.find(item => item.id === id);
  if (!preset) throw new Error(t('provider.unknown', { id }));
  return preset;
}

/** 提供商显示名（当前语言）；Gmail 等品牌名保持原文。 */
export function providerPresetLabel(id: ProviderId): string {
  if (id === 'gmail') return 'Gmail';
  return t(providerPreset(id).labelKey);
}
/** 提供商描述文案（当前语言）。 */
export function providerPresetDescription(id: ProviderId): string {
  return t(providerPreset(id).descriptionKey);
}
/** 提供商授权码提示（当前语言）；未配置返回 undefined。 */
export function providerPresetAuthHint(id: ProviderId): string | undefined {
  const key = providerPreset(id).authHintKey;
  return key ? t(key) : undefined;
}

/** 域名启发式预选卡片；未识别的域返回其他 IMAP。 */
export function detectProvider(email: string): ProviderId {
  const domain = email.trim().toLowerCase().split('@')[1] ?? '';
  if (domain === 'gmail.com' || domain === 'googlemail.com') return 'gmail';
  if (['qq.com', 'foxmail.com', 'vip.qq.com'].includes(domain)) return 'qq';
  if (['163.com', '126.com', 'vip.163.com', 'vip.126.com'].includes(domain)) return '163';
  if (['outlook.com', 'hotmail.com', 'live.com', 'microsoft.com'].includes(domain)) return 'microsoft';
  return 'imap';
}
