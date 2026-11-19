import { create } from 'zustand';
import type { AccountSignature } from '../domain/signature';
import { validateAccountSignature } from '../domain/signature';
import { desktopMail, isDesktop } from '../platform/desktop';
import { t } from '../i18n/core';

/** 每账号签名（读写 + 图片选取）：桌面端能力，浏览器演示明确拒绝。 */
export const useSignatureStore = create<{
  load: (accountId: string) => Promise<AccountSignature>;
  save: (accountId: string, signature: AccountSignature) => Promise<void>;
  pickImage: () => Promise<{ dataUri: string; name: string; size: number } | null>;
}>(() => ({
  load(accountId) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    return desktopMail.accountSignature(accountId).then(validateAccountSignature);
  },
  save(accountId, signature) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    return desktopMail.saveAccountSignature(accountId, signature);
  },
  pickImage() {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    return desktopMail.pickImage();
  },
}));
