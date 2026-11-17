import { t } from '../i18n/core';
/** Compose accepts pasted display names but stores only envelope addresses. */
export function splitRecipients(raw: string): string[] {
  return (raw.match(/(?:"[^"\r\n]*"|[^,;；，\r\n])+/g) ?? [])
    .map(part => part.trim().replace(/^.*<([^<>]+)>$/, '$1').trim())
    .filter(Boolean);
}

export function isRecipientAddress(address: string): boolean {
  const parts = address.split('@');
  if (parts.length !== 2 || address.length > 320 || !/^[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+$/.test(parts[0])) return false;
  const labels = parts[1].split('.');
  return labels.length > 1 && labels.every(label => label.length <= 63 && /^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/.test(label));
}

export function recipientError(fields: string[]): string | null {
  const addresses = fields.flatMap(splitRecipients);
  if (!addresses.length) return t('recipients.needOne');
  if (addresses.some(address => !isRecipientAddress(address))) return t('recipients.fixMarked');
  return null;
}
