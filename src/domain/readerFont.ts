import { t } from '../i18n/core';

/**
 * 阅读正文字号（可调整字号档位）：纯文本正文经 CSS 变量生效，HTML 沙箱正文经
 * mail_message_html 的 font_size 参数写入包装 CSS（Rust 侧白名单 11–24px 回落 13）。
 */
export const READER_FONT_KEY = 'auumail.ui.readerFont.v1';
export const DEFAULT_READER_FONT_PX = 13;

export interface ReaderFontOption { id: 'small' | 'default' | 'large' | 'xlarge'; px: number }

export const READER_FONT_OPTIONS: ReaderFontOption[] = [
  { id: 'small', px: 12 },
  { id: 'default', px: 13 },
  { id: 'large', px: 15 },
  { id: 'xlarge', px: 17 },
];

export function isReaderFontId(value: unknown): value is ReaderFontOption['id'] {
  return value === 'small' || value === 'default' || value === 'large' || value === 'xlarge';
}

export function readerFontPx(id: ReaderFontOption['id']): number {
  return READER_FONT_OPTIONS.find(option => option.id === id)?.px ?? DEFAULT_READER_FONT_PX;
}

export function readerFontLabel(id: ReaderFontOption['id']): string {
  return t(`readerFont.${id}`);
}
