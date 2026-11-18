import { create } from 'zustand';
import { readPreference, writePreference } from '../lib/preferences';

/** 同标题折叠开关：纯界面偏好（默认关闭），持久化键与语言 / 导航偏好同级。 */
const KEY = 'auumail.ui.threads.v1';

interface ThreadViewState {
  groupBySubject: boolean;
  toggle: () => void;
}

const validate = (value: unknown): boolean => {
  if (typeof value !== 'boolean') throw new Error('Invalid thread view preference');
  return value;
};

export const useThreadViewStore = create<ThreadViewState>(set => ({
  groupBySubject: readPreference(KEY, validate) ?? false,
  toggle: () => set(state => {
    const next = !state.groupBySubject;
    writePreference(KEY, next);
    return { groupBySubject: next };
  }),
}));
