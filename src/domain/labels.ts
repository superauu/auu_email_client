import type { MailLabel } from './mail';
import { t } from '../i18n/core';

/**
 * 用户自建标签（桌面端 labels 表 scope='local' 无账号归属的全局标签）：
 * 纯本机注解，无 IMAP 回写；邮件行被同步清理时标签随级联删除。
 * 历史演示标签没有该前缀，仅在真实邮件使用时展示（见 realWorkspace）。
 */
export const USER_LABEL_PREFIX = 'lbl:';
export const LABEL_COLORS = ['teal', 'blue', 'amber'] as const;
export type LabelColor = MailLabel['color'];
export const MAX_LABEL_NAME_CHARS = 64;

export const isUserLabel = (label: MailLabel): boolean => label.id.startsWith(USER_LABEL_PREFIX);

/** 名称 trim 后非空且不超过上限；返回空串表示合法，否则返回当前语言的错误文案。 */
export function labelNameError(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return t('labels.nameRequired');
  if (trimmed.length > MAX_LABEL_NAME_CHARS || [...trimmed].length > MAX_LABEL_NAME_CHARS) return t('labels.nameTooLong');
  return '';
}

export function isLabelColor(color: string): color is LabelColor {
  return (LABEL_COLORS as readonly string[]).includes(color);
}
