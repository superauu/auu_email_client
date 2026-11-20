import { create } from 'zustand';
import type { AccountSignature, SignatureEntry } from '../domain/signature';
import { validateAccountSignature } from '../domain/signature';
import { desktopMail, isDesktop } from '../platform/desktop';
import { t } from '../i18n/core';

/** 编辑态载荷（设置页组装，store 只透传）。 */
export interface SignatureEdit {
  id: string | null;
  name: string;
  form: AccountSignature;
  makeDefault: boolean;
}

/** 多签名（v23）：默认签名读取供撰写注入；列表 / 保存 / 删除供设置页管理。 */
export const useSignatureStore = create<{
  load: (accountId: string) => Promise<AccountSignature>;
  list: (accountId: string) => Promise<SignatureEntry[]>;
  saveEntry: (accountId: string, edit: SignatureEdit) => Promise<SignatureEntry>;
  removeEntry: (accountId: string, id: string) => Promise<void>;
  pickImage: () => Promise<{ dataUri: string; name: string; size: number } | null>;
}>(() => ({
  load(accountId) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    return desktopMail.accountSignature(accountId).then(validateAccountSignature);
  },
  list(accountId) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    return desktopMail.signatureList(accountId);
  },
  saveEntry(accountId, edit) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    return desktopMail.saveSignature(accountId, edit.id, edit.name.trim(), edit.form, edit.makeDefault);
  },
  async removeEntry(accountId, id) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    await desktopMail.deleteSignature(accountId, id);
  },
  pickImage() {
    if (!isDesktop()) return Promise.reject(new Error(t('store.signatureDesktopOnly')));
    return desktopMail.pickImage();
  },
}));
