import { t } from '../i18n/core';

/**
 * 每账号邮件签名（Outlook 式自动追加，v20 迁移镜像）。
 * html 是 compose 消毒后的签名正文，图片以 `data:image/*` 内嵌——发送时由 outbox
 * 转 `cid:` 内联件；两个开关分别控制新邮件与回复 / 转发的追加行为。
 */
export interface AccountSignature {
  html: string;
  useNew: boolean;
  useReply: boolean;
}

/** body_html 上限（与 Rust drafts::MAX_BODY_HTML 对称）：签名图片以 base64 内嵌于 HTML。 */
export const MAX_BODY_HTML = 2_000_000;
/** 内联位图单图上限（解码后字节；与 Rust signature::MAX_SIGNATURE_IMAGE 对称）。
 *  签名图片选取与正文 Ctrl+V 粘贴共用同一上限。 */
export const MAX_INLINE_IMAGE = 1024 * 1024;
/** 签名单图上限（解码后字节；与 Rust signature::MAX_SIGNATURE_IMAGE 对称）。 */
export const MAX_SIGNATURE_IMAGE = MAX_INLINE_IMAGE;

/** 签名列表条目（v23 多签名）：id 为不透明字符串；isDefault 供撰写注入与设置页星标。 */
export interface SignatureEntry {
  id: string;
  name: string;
  html: string;
  useNew: boolean;
  useReply: boolean;
  isDefault: boolean;
}

/** 签名数量上限（与 Rust MAX_SIGNATURES_PER_ACCOUNT 对称）。 */
export const MAX_SIGNATURES_PER_ACCOUNT = 10;
/** 签名名称长度上限。 */
export const MAX_SIGNATURE_NAME_CHARS = 100;

export function validateAccountSignature(raw: unknown): AccountSignature {
  if (!raw || typeof raw !== 'object') throw new Error(t('signature.invalidData'));
  const value = raw as Record<string, unknown>;
  if (typeof value.html !== 'string' || value.html.length > MAX_BODY_HTML) throw new Error(t('signature.tooLarge'));
  if (typeof value.useNew !== 'boolean' || typeof value.useReply !== 'boolean') throw new Error(t('signature.invalidData'));
  return { html: value.html, useNew: value.useNew, useReply: value.useReply };
}
