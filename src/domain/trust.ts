import { t } from '../i18n/core';

/** 远程图片信任条目（v21 迁移镜像）：kind='sender' 完整发件地址 / 'domain' 发件域名。 */
export interface TrustedEntry {
  kind: 'sender' | 'domain';
  value: string;
  createdAt: string;
}

/** 发件地址 → 域名（最后一个 @ 之后，小写）。 */
export function senderDomain(email: string): string {
  const at = email.lastIndexOf('@');
  return at >= 0 ? email.slice(at + 1).toLowerCase() : '';
}

/** 发件人是否已被信任：地址精确匹配，或其域名在信任域名列表中（精确匹配，不含子域）。 */
export function isTrustedSender(email: string, entries: TrustedEntry[] | null): boolean {
  if (!entries?.length) return false;
  const address = email.trim().toLowerCase();
  if (!address) return false;
  const domain = senderDomain(address);
  return entries.some(entry =>
    entry.kind === 'sender' ? entry.value === address : entry.kind === 'domain' && domain !== '' && entry.value === domain,
  );
}

export function validateTrustKind(kind: string): 'sender' | 'domain' {
  if (kind === 'sender' || kind === 'domain') return kind;
  throw new Error(t('trust.invalidData'));
}

export function validateTrustedEntries(raw: unknown): TrustedEntry[] {
  if (!Array.isArray(raw)) throw new Error(t('trust.invalidData'));
  const entries: TrustedEntry[] = [];
  for (const item of raw) {
    if (!item || typeof item !== 'object') throw new Error(t('trust.invalidData'));
    const value = item as Record<string, unknown>;
    if (typeof value.kind !== 'string' || !['sender', 'domain'].includes(value.kind)) throw new Error(t('trust.invalidData'));
    if (typeof value.value !== 'string' || !value.value || value.value.length > 320) throw new Error(t('trust.invalidData'));
    if (typeof value.createdAt !== 'string') throw new Error(t('trust.invalidData'));
    entries.push({ kind: value.kind as 'sender' | 'domain', value: value.value, createdAt: value.createdAt });
  }
  return entries;
}
