import { create } from 'zustand';
import { readPreference, writePreference } from '../lib/preferences';
import { isReaderFontId, readerFontPx, READER_FONT_KEY, type ReaderFontOption } from '../domain/readerFont';
import { t } from '../i18n/core';

const saved = readPreference(READER_FONT_KEY, raw => {
  if (!isReaderFontId(raw)) throw new Error('Invalid reader font');
  return raw;
});

interface ReaderFontState {
  fontId: ReaderFontOption['id'];
  warning: string | null;
  setFontId: (id: ReaderFontOption['id']) => void;
  clearWarning: () => void;
}

/** 把当前档位写到根元素 CSS 变量（纯文本正文用）；HTML 沙箱在渲染时读取档位。 */
function applyFontSize(id: ReaderFontOption['id']) {
  document.documentElement.style.setProperty('--reader-font-size', `${readerFontPx(id)}px`);
}

/** 模块加载即应用（首屏前生效），与外观初始化同层级。 */
if (saved) applyFontSize(saved);

export const useReaderFontStore = create<ReaderFontState>(set => ({
  fontId: saved ?? 'default',
  warning: null,
  setFontId(id) {
    applyFontSize(id);
    const persisted = writePreference(READER_FONT_KEY, id);
    set({ fontId: id, ...(!persisted ? { warning: t('readerFont.prefNotSaved') } : {}) });
  },
  clearWarning() { set({ warning: null }); },
}));
