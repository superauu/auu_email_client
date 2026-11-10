/** localStorage is for non-secret UI preferences and explicitly labelled demo drafts only. */
export function readPreference<T>(key: string, validate: (value: unknown) => T): T | undefined {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? undefined : validate(JSON.parse(raw));
  } catch { return undefined; }
}

export function writePreference(key: string, value: unknown): boolean {
  try { localStorage.setItem(key, JSON.stringify(value)); return true; }
  catch { return false; }
}
