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
  label: string;
  description: string;
  available: boolean;
  /** OAuth 走 mail_oauth_start；密码 IMAP 走账号向导保存路径。 */
  oauth: boolean;
  endpoints?: ProviderEndpoints;
  /** 密码提示，例如国内邮箱使用授权码而非登录密码。 */
  authHint?: string;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'gmail',
    label: 'Gmail',
    description: 'Google 邮箱，授权在系统浏览器完成',
    available: true,
    oauth: true,
  },
  {
    id: 'microsoft',
    label: 'Outlook / Microsoft 365',
    description: 'OAuth 应用注册完成后开放',
    available: false,
    oauth: true,
  },
  {
    id: 'qq',
    label: 'QQ 邮箱',
    description: '使用 IMAP/SMTP 授权码登录',
    available: true,
    oauth: false,
    endpoints: { imapHost: 'imap.qq.com', imapPort: 993, imapSecurity: 'tls', smtpHost: 'smtp.qq.com', smtpPort: 465 },
    authHint: '需要在 QQ 邮箱网页版开启 IMAP 服务，并使用生成的授权码（不是 QQ 密码）。',
  },
  {
    id: '163',
    label: '网易 163 邮箱',
    description: '使用 IMAP/SMTP 授权码登录',
    available: true,
    oauth: false,
    endpoints: { imapHost: 'imap.163.com', imapPort: 993, imapSecurity: 'tls', smtpHost: 'smtp.163.com', smtpPort: 465 },
    authHint: '需要在 163 邮箱网页版开启 IMAP 服务，并使用客户端授权密码。',
  },
  {
    id: 'pop3',
    label: 'POP3 邮箱',
    description: '手动填写 POP3 / SMTP 服务器',
    available: true,
    oauth: false,
  },
  {
    id: 'imap',
    label: '其他邮箱',
    description: '手动填写 IMAP / SMTP 服务器',
    available: true,
    oauth: false,
  },
];

export function providerPreset(id: ProviderId): ProviderPreset {
  const preset = PROVIDER_PRESETS.find(item => item.id === id);
  if (!preset) throw new Error(`未知邮箱提供商：${id}`);
  return preset;
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
