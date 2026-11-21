import { createContext, useContext, useLayoutEffect } from 'react';

export interface SettingsGuard {
  dirty: boolean;
  busy: boolean;
}

export const SettingsNavigation = createContext<{
  guards: Set<SettingsGuard>;
  request: (action: () => void) => void;
} | null>(null);

export function useSettingsNavigation(dirty: boolean, busy: boolean) {
  const navigation = useContext(SettingsNavigation);
  const guards = navigation?.guards;
  useLayoutEffect(() => {
    if (!guards) return;
    const guard = { dirty, busy };
    guards.add(guard);
    return () => { guards.delete(guard); };
  }, [guards, dirty, busy]);
  return navigation?.request;
}
