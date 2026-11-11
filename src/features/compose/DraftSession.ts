import type { DraftSaveOptions, MailDraft } from '../../domain/mail';
import { errorMessage } from '../../domain/errors';
import { validateDraft } from '../../domain/drafts';

export interface DraftSessionState {
  draft: MailDraft;
  dirty: boolean;
  saving: boolean;
  saved: boolean;
  error: string | null;
}
type SaveDraft = (draft: MailDraft, options: DraftSaveOptions) => Promise<MailDraft>;

/** Serializes writes and coalesces edits made during an in-flight save. */
export class DraftSession {
  private state: DraftSessionState;
  private listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private inFlight: Promise<void> | null = null;
  private editVersion = 0;
  private revision: number;
  private owner: string;
  private paused = false;

  constructor(initial: MailDraft, private readonly write: SaveDraft, private readonly debounceMs = 800) {
    this.revision = initial.revision ?? 0;
    this.owner = initial.accountId;
    this.state = { draft: { ...initial }, dirty: false, saving: false, saved: this.revision > 0, error: null };
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private publish(changes: Partial<DraftSessionState>) {
    this.state = { ...this.state, ...changes };
    for (const listener of this.listeners) listener();
  }
  update(changes: Partial<Pick<MailDraft, 'accountId' | 'to' | 'subject' | 'bodyText'>>) {
    this.editVersion += 1;
    this.publish({ draft: { ...this.state.draft, ...changes }, dirty: true, saved: false, error: null });
    this.schedule();
  }
  cancelScheduledSave = () => { clearTimeout(this.timer); this.timer = undefined; };
  pause() { this.paused = true; this.cancelScheduledSave(); }
  resume() { this.paused = false; this.schedule(); }
  needsProtection = () => this.state.dirty || this.state.saving;
  private schedule() {
    this.cancelScheduledSave();
    if (!this.paused && this.state.dirty && !this.inFlight) {
      this.timer = setTimeout(() => { this.timer = undefined; void this.start(false); }, this.debounceMs);
    }
  }
  async flush(): Promise<boolean> {
    this.cancelScheduledSave();
    if (this.inFlight) { await this.inFlight; if (this.state.error) return false; }
    if (!this.state.dirty && this.state.saved) return true;
    await this.start(true);
    return !this.state.dirty && this.state.saved && !this.state.error;
  }
  private start(manual: boolean): Promise<void> {
    if (this.inFlight) return this.inFlight;
    this.publish({ saving: true, error: null });
    this.inFlight = this.drain(manual).finally(() => { this.inFlight = null; this.publish({ saving: false }); });
    return this.inFlight;
  }
  private async drain(manual: boolean) {
    try {
      do {
        const version = this.editVersion;
        const input = { ...this.state.draft, revision: this.revision, updatedAt: new Date().toISOString() };
        const saved = validateDraft(await this.write(input, { previousAccountId: this.owner }));
        if (saved.id !== input.id || saved.accountId !== input.accountId || (saved.revision ?? 0) !== this.revision + 1) throw new Error('草稿保存结果无效，请保留当前内容。');
        this.revision = saved.revision!;
        this.owner = saved.accountId;
        const dirty = this.editVersion !== version;
        this.publish({ draft: { ...this.state.draft, revision: saved.revision, updatedAt: saved.updatedAt }, dirty, saved: !dirty });
        if (this.paused && !manual) break;
      } while (this.state.dirty);
    } catch (error) { this.publish({ dirty: true, saved: false, error: errorMessage(error) }); }
  }
}
