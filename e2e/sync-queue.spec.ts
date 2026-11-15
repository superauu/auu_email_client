import { expect, test, type Page } from '@playwright/test';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { MailAccount, MailDraft, MailMessage, MessagePatch } from '../src/domain/mail';
import type { SyncJob } from '../src/domain/sync';

/** Test-only IPC harness. Validates UI behavior, not a real server or the Rust queue. */
async function mockDesktop(page: Page, messageChanges = false, recoveryMode?: 'legacy' | 'unknown' | 'queued' | 'paged' | 'fingerprint' | 'unanchored' | 'segmented', completion = false) {
  const accounts: MailAccount[] = [1, 2, 3].map(index => ({ ...demoWorkspace.accounts[0], id: `real-${index}`, name: `测试邮箱 ${index}`, email: `test${index}@mail.example`, provider: 'imap', status: 'offline', capabilities: { ...demoWorkspace.accounts[0].capabilities, send: true } }));
  await page.addInitScript(({ fixture, accounts, messageChanges, recoveryMode, completion }) => {
    const workspace = { ...fixture, accounts: [...fixture.accounts, ...accounts] };
    if (messageChanges || completion) workspace.messages.unshift({ ...fixture.messages[0], id: 'real-message', accountId: 'real-1', unread: false, subject: '离线状态测试', stateRevision: 1, changes: [{ field: 'starred', state: 'conflict', error: 'UIDVALIDITY 已改变，请核对服务器。' }] });
    if (recoveryMode) { const message = workspace.messages.find(message => message.id === 'real-message')!; message.recovery = { state: recoveryMode === 'queued' ? 'queued' : 'available', detail: '旧缓存或未知归档需要远端核对。', location: null, evidence: null }; if (recoveryMode === 'unknown') { message.mailbox = 'archive'; message.changes = [{ field: 'archive', state: 'unknown', error: '归档结果待核对，不会自动重试。' }]; } }
    if (recoveryMode === 'fingerprint' || recoveryMode === 'unanchored') { const message = workspace.messages.find(message => message.id === 'real-message')!; message.recovery!.fingerprint = { eligible: recoveryMode === 'fingerprint', reason: recoveryMode === 'fingerprint' ? '可信原定位仍在，可安全核对原文。' : '缺少可信原定位，不能升级为历史原文指纹。' }; }
    if (completion) {
      const message = workspace.messages.find(message => message.id === 'real-message')!;
      message.attachments = [{index:0,name:'report.txt',size:1024,mediaType:'text/plain'}];
      message.folderIds=['project-1'];
      workspace.messages.push({...message,id:'server-draft',mailbox:'drafts',subject:'服务器草稿测试',changes:[],attachments:[]});
      workspace.messages.push({...fixture.messages[1], id:'plain-link', accountId: fixture.accounts[0].id, subject:'纯文本链接测试', bodyText:'请联系 help@anoma.ly 或访问 https://example.com/help。', unread:false, changes:[], attachments:[]});
    }
    const imapSettings = new Map<string,{syncAllFolders:boolean;historyEnabled:boolean;autoSyncSeconds:number}>();
    let settingFails = completion; let exportCalls = 0; let oauthState = 'waiting';
    const htmlRequests: Array<{ id: string; allow: boolean }> = []; const openCalls: string[] = [];
    const outboxActions: Array<{ id: string; action: string }> = [];
    let outboxState: 'queued' | 'sending' | 'unknown' | 'failed' | 'sent' | null = null;
    const outboxEntry = () => outboxState === null ? null : { id: 'ob-1', draftId: 'sent-draft', accountId: 'real-1', messageId: '<m@auumail.local>', state: outboxState, attempts: 1, lastError: outboxState === 'unknown' ? '邮件已提交给服务器，但未能确认投递结果' : outboxState === 'failed' ? '已按仅保留本机处理；服务器是否投递未被确认，不会自动重试' : null, queuedAt: new Date().toISOString(), updatedAt: new Date().toISOString(), resolvedBy: null };
    const recoveryActions = new Map<string, string>();
    let nextRecovery: 'located' | 'source-present' | 'unresolved' | 'scanning' = 'located';
    let segmentPhase: 'collecting' | 'validating' | 'located' | 'unresolved' = 'collecting';
    let revision = 0; let sequence = 0;
    const jobs: SyncJob[] = []; const drafts = new Map<string, MailDraft>();
    const windowCalls: string[] = [];
    const snapshot = () => structuredClone({ revision, jobs });
    function project(draft: MailDraft): MailMessage {
      const account = workspace.accounts.find(account => account.id === draft.accountId)!;
      return { id: draft.id, accountId: draft.accountId, threadId: draft.replyToId ?? draft.id, mailbox: 'drafts', from: { name: account.name, email: account.email }, to: draft.to ? [draft.to] : [], subject: draft.subject || '未命名草稿', preview: draft.bodyText.slice(0, 120), bodyText: draft.bodyText, receivedAt: draft.updatedAt, unread: false, starred: false, labelIds: [], draft: { ...draft } };
    }
    const invoke = async (command: string, input: Record<string, unknown> = {}) => {
      switch (command) {
        case 'plugin:window|start_dragging': case 'plugin:window|toggle_maximize': case 'plugin:window|minimize': windowCalls.push(command);return;
        case 'get_app_info': return { version: '0.1.0', dataDirectory: 'test-only', skinDirectory: 'test-only/skins', databaseReady: true };
        case 'list_user_skins': return { skins: [], warnings: [] };
        case 'mail_seed_workspace': case 'mail_import_legacy_drafts': case 'plugin:event|unlisten': return;
        case 'mail_load_workspace': return { accounts: workspace.accounts, labels: workspace.labels, events: workspace.events, folders: completion ? [{id:'project-1',accountId:'real-1',name:'项目邮件',kind:'other',available:true}] : [], syncStates: [] };
        case 'mail_page_messages': return { items: workspace.messages, nextCursor: null };
        case 'mail_list_drafts': return [...drafts.values()].map(project);
        case 'mail_request_recovery': {
          const message = workspace.messages.find(message => message.id === input.messageId && message.accountId === input.accountId)!;
          if (message.stateRevision !== input.revision) throw new Error('邮件版本已改变');
          message.stateRevision = (message.stateRevision ?? 0) + 1;
          if (input.action !== 'continue') recoveryActions.set(message.id, input.action as string);
          message.recovery = { ...message.recovery!, state: 'queued', detail: '已登记核对请求，等待账号同步。' }; return structuredClone(message);
        }
        case 'mail_update_message': {
          const message = workspace.messages.find(message => message.id === input.messageId && message.accountId === input.accountId)!;
          const patch = input.patch as MessagePatch;
          if (patch.unread !== undefined) message.unread = patch.unread;
          if (patch.starred !== undefined) message.starred = patch.starred;
          if (patch.archive) message.mailbox = 'archive';
          if (message.accountId.startsWith('real-')) {
            for (const field of ['unread', 'starred', 'archive'] as const) if (patch[field] !== undefined) message.changes = [...(message.changes ?? []).filter(change => change.field !== field), { field, state: 'queued', error: null }];
          }
          message.stateRevision = (message.stateRevision ?? 0) + 1;
          return structuredClone(message);
        }
        case 'mail_keep_message_local': {
          const message = workspace.messages.find(message => message.id === input.messageId && message.accountId === input.accountId)!;
          message.changes = message.changes?.map(change => ({ ...change, state: 'local', error: null }));
          message.stateRevision = (message.stateRevision ?? 0) + 1; return structuredClone(message);
        }
        case 'mail_save_draft': {
          const draft = input.draft as MailDraft;
          const saved = { ...draft, revision: (draft.revision ?? 0) + 1, updatedAt: new Date().toISOString() };
          drafts.set(saved.id, saved); return project(saved);
        }
        case 'mail_imap_settings': return imapSettings.get(input.accountId as string) ?? {syncAllFolders:true,historyEnabled:false,autoSyncSeconds:0};
        case 'mail_save_imap_settings': if(settingFails){settingFails=false;throw new Error('设置暂时未保存');} imapSettings.set(input.accountId as string,input.settings as {syncAllFolders:boolean;historyEnabled:boolean;autoSyncSeconds:number});return;
        case 'mail_account_config': {const account=accounts.find(account=>account.id===input.id)!; return {id:account.id,name:account.name,email:account.email,imapHost:'imap.example.test',imapPort:993,imapSecurity:'tls',smtpHost:'',smtpPort:465,receiveOnly:true,authUser:account.email,avatar:'T',color:'teal'};}
        case 'mail_verify_account': return {imap:{ok:true,skipped:false,error:null,code:null},smtp:{ok:false,skipped:true,error:null,code:null},folderCount:2};
        case 'mail_oauth_start': oauthState='waiting';return 'flow-test';
        case 'mail_oauth_status': return {id:'flow-test',state:oauthState,accountId:null,error:null,folderCount:null};
        case 'mail_oauth_cancel': oauthState='cancelled';return {id:'flow-test',state:oauthState,accountId:null,error:null,folderCount:null};
        case 'mail_export_attachment': exportCalls++;if(exportCalls===2) throw new Error('原文缓存已被清理，请重新下载');return exportCalls>2;
        case 'mail_send_draft': {
          const draft = drafts.get(input.draftId as string);
          if (!draft) throw new Error('草稿不存在');
          if (draft.revision !== input.revision) throw new Error('草稿已被修改，请保存后重新发送');
          outboxState = 'queued';
          return outboxEntry()!;
        }
        case 'mail_outbox_list': return outboxEntry() ? [outboxEntry()!] : [];
        case 'mail_message_html_fragment': return '<p>引用的原文 HTML</p>';
        case 'mail_outbox_cancel': outboxState = null; return outboxEntry()!;
        case 'mail_outbox_resolve': outboxActions.push({ id: input.id as string, action: input.action as string }); outboxState = input.action === 'retry' ? 'queued' : 'failed'; return outboxEntry()!;
        case 'mail_retry_outbox': outboxState = 'queued'; return outboxEntry()!;
        case 'mail_message_html': {
          const message = workspace.messages.find(message => message.id === input.messageId && message.accountId === input.accountId);
          if (!message) throw new Error('邮件不存在或不属于此账号');
          htmlRequests.push({ id: message.id, allow: Boolean(input.allowRemoteImages) });
          const images = input.allowRemoteImages ? 'img-src https: data:' : 'img-src \'none\'';
          return { html: `<!DOCTYPE html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; ${images}"></head><body><p>富文本正文（${input.allowRemoteImages ? '远程图片已允许' : '远程图片被拦截'}）</p><a href="https://example.test/receipt">收据链接</a></body></html>` };
        }
        case 'open_external': openCalls.push(input.url as string); return;
        case 'mail_sync_jobs': return snapshot();
        case 'mail_enqueue_sync': {
          for (const id of input.ids as string[]) {
            if (jobs.some(job => job.accountId === id && ['queued', 'running'].includes(job.status))) continue;
            const running = jobs.filter(job => job.status === 'running').length < 2;
            jobs.push({ id: `job-${++sequence}`, accountId: id, status: running ? 'running' : 'queued', queuedAt: new Date().toISOString(), startedAt: running ? new Date().toISOString() : null, finishedAt: null, outcome: null, error: null });
            revision += 1;
          }
          return snapshot();
        }
        case 'mail_cancel_sync': {
          const job = jobs.find(job => job.id === input.jobId)!;
          if (job.status === 'running') {job.cancelRequested=true;revision+=1;return snapshot();}
          if (job.status !== 'queued') throw new Error('Already finished');
          job.status = 'cancelled'; job.finishedAt = new Date().toISOString(); revision += 1; return snapshot();
        }
        case 'plugin:event|listen': return 1;
        default: throw new Error(`Unexpected test IPC: ${command}`);
      }
    };
    Object.defineProperty(window, 'isTauri', { value: true });
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: { invoke, metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }, transformCallback: () => 1, unregisterCallback: () => {} } });
    Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', { value: { unregisterListener: () => {} } });
    Object.defineProperty(window, '__syncTest', { value: {
      windowCalls: () => [...windowCalls],
      openCalls: () => [...openCalls],
      htmlRequests: () => [...htmlRequests],
      outboxState(state: 'queued' | 'sending' | 'unknown' | 'failed' | 'sent' | null) { outboxState = state; },
      outboxActions: () => [...outboxActions],
      recoveryResult(result: 'located' | 'source-present' | 'unresolved' | 'scanning') { nextRecovery = result; },
      segmentPhase(phase: 'collecting' | 'validating' | 'located' | 'unresolved') { segmentPhase = phase; },
      finish(id: string, failure: boolean) {
        const job = jobs.find(job => job.accountId === id && job.status === 'running')!;
        job.status = failure ? 'failed' : 'succeeded'; job.finishedAt = new Date().toISOString();
        job.error = failure ? { code: 'network', message: '测试连接失败', retryable: true } : null;
        for (const message of workspace.messages.filter(message => message.accountId === id)) {
          if (message.recovery?.state === 'queued') {
            const action = recoveryActions.get(message.id) ?? 'check';
            if (recoveryMode === 'fingerprint') {
              message.recovery = { ...message.recovery, operation: 'fingerprint', state: failure ? 'failed' : action === 'fingerprint' ? 'located' : 'restored', detail: failure ? '指纹核对失败，本机数据保留。' : action === 'fingerprint' ? '原定位和两次原文已核对，请明确补齐指纹。' : '可信完整原文指纹已补齐；本地修改与远端定位保持。', location: { folder: 'INBOX', kind: 'inbox', uidValidity: 10, uid: 1 }, fingerprint: action === 'confirm-fingerprint' && !failure ? null : message.recovery.fingerprint };
              message.stateRevision = (message.stateRevision ?? 0) + 1; continue;
            }
            if (recoveryMode === 'segmented') {
              message.recovery = segmentPhase === 'unresolved' ? { state: 'unresolved', detail: '全段复核发现集合变化，未绑定。', location: null, evidence: null } : segmentPhase === 'located' ? { state: action === 'check' ? 'located' : 'restored', detail: action === 'check' ? '全部分段与集合复核完成，请确认。' : '分段恢复已确认，本地修改保留。', location: { folder: 'INBOX', kind: 'inbox', uidValidity: 20, uid: 50001 }, evidence: 'raw-sha256' } : { state: failure ? 'failed' : 'scanning', detail: '分段未完成，不能确认全局唯一性。', location: null, evidence: 'raw-sha256', progress: { checked: 50001, total: 50001, matches: 1, segments: { completed: segmentPhase === 'collecting' ? 1 : 0, total: 2, phase: segmentPhase } } };
              message.stateRevision = (message.stateRevision ?? 0) + 1; continue;
            }
            if (recoveryMode === 'paged') {
              message.recovery = failure ? { ...message.recovery, state: 'failed', detail: '分页核对连接失败，已完成页面仍保留。' } : nextRecovery === 'scanning' ? { state: 'scanning', detail: '分页未完成，不能确认唯一性。', location: null, evidence: 'raw-sha256', progress: { checked: 8, total: 101, matches: 1 }, crossUidValidity: true } : nextRecovery === 'unresolved' ? { state: 'unresolved', detail: '后续页发现重复匹配，未绑定新 UID。', location: null, evidence: null, progress: null } : { state: action === 'check' ? 'located' : 'restored', detail: action === 'check' ? '完整扫描已匹配原文指纹，请确认新定位。' : '跨代定位已恢复，本地修改保留。', location: { folder: 'INBOX', kind: 'inbox', uidValidity: 20, uid: 200 }, evidence: 'raw-sha256', progress: null, crossUidValidity: true };
              message.stateRevision = (message.stateRevision ?? 0) + 1; continue;
            }
            message.recovery = failure ? { ...message.recovery, state: 'failed', detail: '核对连接失败，本机内容已保留。' } : action === 'check' ? { state: nextRecovery, detail: nextRecovery === 'unresolved' ? '发现重复匹配，归档结果仍不确定。' : nextRecovery === 'source-present' ? '原邮件仍在源文件夹，目标未找到匹配。' : '已找到内容字段相符的定位，请明确确认。', location: { folder: recoveryMode === 'unknown' && nextRecovery === 'located' ? 'Archive' : 'INBOX', kind: recoveryMode === 'unknown' ? 'archive' : 'inbox', uidValidity: 20, uid: 7 }, evidence: 'cached-fields' } : { ...message.recovery, state: 'restored', detail: action === 'retry-archive' ? '已明确重新排队归档；下次手动同步尝试移动。' : '定位已恢复，本机修改保留；下次手动同步处理。' };
            if (!failure && action !== 'check') message.changes = message.changes?.filter(change => action !== 'confirm' || change.field !== 'archive').map(change => ({ ...change, state: 'queued', error: null }));
            message.stateRevision = (message.stateRevision ?? 0) + 1; continue;
          }
          message.changes = failure ? message.changes?.map(change => change.state === 'queued' ? { ...change, state: 'failed', error: '标记回写未确认，下次同步可重试。' } : change) : [];
          message.stateRevision = (message.stateRevision ?? 0) + 1;
        }
        job.outcome = failure ? null : { folders: 2, fetchedMessages: 1, syncedAt: new Date().toISOString(), warnings: [] };
        const next = jobs.find(job => job.status === 'queued');
        if (next) { next.status = 'running'; next.startedAt = new Date().toISOString(); }
        revision += 1;
      },
    } });
  }, { fixture: demoWorkspace, accounts, messageChanges, recoveryMode, completion });
}

async function finishRecovery(page: Page, failure = false) {
  await expect(page.getByRole('button', { name: '远端核对处理中', exact: true })).toBeDisabled();
  await page.evaluate(failure => (window as unknown as { __syncTest: { finish: (id: string, failure: boolean) => void } }).__syncTest.finish('real-1', failure), failure);
}

async function nextPageResult(page: Page, result: string) {
  await page.evaluate(value => (window as unknown as { __syncTest: { recoveryResult: (value: string) => void } }).__syncTest.recoveryResult(value), result);
}

test('trusted legacy fingerprint refill requires explicit confirmation and keeps local edits', async ({ page }, info) => {
  await mockDesktop(page, true, 'fingerprint'); await page.goto('/'); await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await expect(page.getByRole('button', { name: '确认补齐完整原文指纹', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '核对并补齐原文指纹', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: '确认补齐完整原文指纹', exact: true })).toBeEnabled();
  await page.screenshot({ path: info.outputPath('fingerprint-refill-review.png') });
  await page.getByRole('button', { name: '确认补齐完整原文指纹', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('可信完整原文指纹已补齐；本地修改与远端定位保持。')).toBeVisible();
  await expect(page.getByRole('button', { name: '切换当前邮件星标', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('unanchored cached content cannot be upgraded to a historical raw fingerprint', async ({ page }) => {
  await mockDesktop(page, true, 'unanchored'); await page.goto('/'); await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await expect(page.getByText('缺少可信原定位，不能升级为历史原文指纹。')).toBeVisible();
  await expect(page.getByRole('button', { name: '核对并补齐原文指纹', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: '确认补齐完整原文指纹', exact: true })).toHaveCount(0);
});

test('over-limit range collection cannot bind before the final all-segment validation', async ({ page }, info) => {
  await mockDesktop(page, true, 'segmented'); await page.goto('/'); await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await page.getByRole('button', { name: '核对远端定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText(/分段正文核对：1 \/ 2，已检查 50001 个候选/)).toBeVisible();
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as unknown as { __syncTest: { segmentPhase: (value: string) => void } }).__syncTest.segmentPhase('validating'));
  await page.getByRole('button', { name: '继续分段核对', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByRole('button', { name: '继续全段复核', exact: true })).toBeEnabled();
  await page.screenshot({ path: info.outputPath('segmented-validation.png') });
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as unknown as { __syncTest: { segmentPhase: (value: string) => void } }).__syncTest.segmentPhase('unresolved'));
  await page.getByRole('button', { name: '继续全段复核', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('全段复核发现集合变化，未绑定。')).toBeVisible();
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
});

test('paged cross-generation recovery retains progress and requires a complete confirmation scan', async ({ page }, info) => {
  await mockDesktop(page, true, 'paged'); await page.goto('/');
  await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await nextPageResult(page, 'scanning'); await page.getByRole('button', { name: '核对远端定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText(/候选核对进度：8 \/ 101/)).toBeVisible();
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('paged-recovery-progress.png') });
  await page.getByRole('button', { name: '继续核对下一页', exact: true }).click(); await finishRecovery(page, true);
  await expect(page.getByText('分页核对连接失败，已完成页面仍保留。')).toBeVisible();
  await nextPageResult(page, 'located'); await page.getByRole('button', { name: '继续核对下一页', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('核对位置：INBOX · UIDVALIDITY 20 · UID 200')).toBeVisible();
  await nextPageResult(page, 'scanning'); await page.getByRole('button', { name: '确认恢复定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await nextPageResult(page, 'located'); await page.getByRole('button', { name: '继续核对下一页', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('跨代定位已恢复，本地修改保留。')).toBeVisible();
  await expect(page.getByRole('button', { name: '切换当前邮件星标', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('a match on the first page cannot be confirmed when a later page finds duplicates', async ({ page }) => {
  await mockDesktop(page, true, 'paged'); await page.goto('/'); await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await nextPageResult(page, 'scanning'); await page.getByRole('button', { name: '核对远端定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await nextPageResult(page, 'unresolved'); await page.getByRole('button', { name: '继续核对下一页', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('后续页发现重复匹配，未绑定新 UID。')).toBeVisible();
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: '切换当前邮件星标', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('legacy cache recovery requires a remote check and a second explicit confirmation', async ({ page }, info) => {
  await mockDesktop(page, true, 'legacy'); await page.goto('/');
  await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '核对远端定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('核对位置：INBOX · UIDVALIDITY 20 · UID 7')).toBeVisible();
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toBeEnabled();
  await page.screenshot({ path: info.outputPath('legacy-recovery-review.png') });
  await page.getByRole('button', { name: '确认恢复定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('定位已恢复，本机修改保留；下次手动同步处理。')).toBeVisible();
});

test('unknown archive stays unresolved for duplicates and only explicit retry queues another move', async ({ page }) => {
  await mockDesktop(page, true, 'unknown'); await page.goto('/');
  await page.getByRole('navigation', { name: '邮件文件夹' }).getByRole('button', { name: '归档' }).click();
  await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await page.evaluate(() => (window as unknown as { __syncTest: { recoveryResult: (value: string) => void } }).__syncTest.recoveryResult('unresolved'));
  await page.getByRole('button', { name: '核对远端定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('发现重复匹配，归档结果仍不确定。')).toBeVisible();
  await expect(page.getByRole('button', { name: '重新排队归档', exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as unknown as { __syncTest: { recoveryResult: (value: string) => void } }).__syncTest.recoveryResult('source-present'));
  await page.getByRole('button', { name: '核对远端定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByRole('button', { name: '重新排队归档', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: '重新排队归档', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByText('已明确重新排队归档；下次手动同步尝试移动。')).toBeVisible();
});

test('a persisted queued check can restart after a failed connection', async ({ page }) => {
  await mockDesktop(page, true, 'queued'); await page.goto('/');
  await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await page.getByRole('button', { name: '开始已登记核对', exact: true }).click(); await finishRecovery(page, true);
  await expect(page.getByText('核对连接失败，本机内容已保留。')).toBeVisible();
  await page.getByRole('button', { name: '核对远端定位', exact: true }).click(); await finishRecovery(page);
  await expect(page.getByRole('button', { name: '确认恢复定位', exact: true })).toBeEnabled();
});

test('local conflicts are explicit and pending flags can be retried without leaving the reader', async ({ page }, info) => {
  await mockDesktop(page, true); await page.goto('/');
  await page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true }).click();
  await expect(page.getByText('UIDVALIDITY 已改变，请核对服务器。')).toBeVisible();
  await page.getByRole('button', { name: '仅保留本机', exact: true }).click();
  await expect(page.getByText('已保存在本机', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: '同步此账号', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '标为未读', exact: true }).click();
  await expect(page.getByText('本机已保存 · 等待同步此账号回写')).toBeVisible();
  await page.getByRole('button', { name: '同步此账号', exact: true }).click();
  await expect(page.getByRole('button', { name: '账号同步中', exact: true })).toBeDisabled();
  await page.evaluate(() => (window as unknown as { __syncTest: { finish: (id: string, failure: boolean) => void } }).__syncTest.finish('real-1', true));
  await expect(page.getByText('标记回写未确认，下次同步可重试。')).toBeVisible();
  await page.screenshot({ path: info.outputPath('message-writeback-failed.png') });
  await expect(page.getByRole('button', { name: '同步此账号', exact: true })).toBeEnabled();
});

test('multi-account progress persists outside settings and drafts remain editable', async ({ page }, info) => {
  await mockDesktop(page); await page.goto('/');
  await expect(page.getByRole('heading', { name: /^收件箱\s*6$/ })).toBeVisible();
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('button', { name: '同步全部', exact: true }).click();
  await expect(page.getByText('2 个账号同步中 · 1 个排队中。关闭设置后任务继续运行。')).toBeVisible();
  await expect(page.getByRole('button', { name: '同步 测试邮箱 1', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '关闭对话框', exact: true }).click();
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await page.getByLabel('发件人', { exact: true }).selectOption('real-2');
  await page.getByRole('textbox', { name: '邮件正文' }).fill('同步期间仍然可以编辑和保存草稿');
  await expect(page.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  await page.getByRole('button', { name: '关闭撰写并返回列表', exact: true }).click();
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await expect(page.getByRole('button', { name: '取消排队 测试邮箱 3', exact: true })).toBeVisible();
  await page.screenshot({ path: info.outputPath('sync-queue.png') });
});

test('one account failing leaves the others running and supports queued retry cancellation', async ({ page }) => {
  await mockDesktop(page); await page.goto('/');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('button', { name: '同步全部', exact: true }).click();
  await expect(page.getByRole('button', { name: '取消排队 测试邮箱 3', exact: true })).toBeVisible();
  await page.evaluate(() => (window as unknown as { __syncTest: { finish: (id: string, failure: boolean) => void } }).__syncTest.finish('real-1', true));
  await expect(page.locator('.sync-note', { hasText: '同步失败：测试连接失败' })).toBeVisible();
  await expect(page.getByRole('button', { name: '同步 测试邮箱 1', exact: true })).toBeEnabled();
  await expect(page.getByRole('button', { name: '同步 测试邮箱 2', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '同步 测试邮箱 1', exact: true }).click();
  await page.getByRole('button', { name: '取消排队 测试邮箱 1', exact: true }).click();
  await expect(page.getByText(/已取消排队/)).toBeVisible();
  await expect(page.getByRole('button', { name: '同步 测试邮箱 1', exact: true })).toBeEnabled();
});

test('IMAP settings preserve edits after failure and remain scoped to one account', async ({page},info) => {
  await mockDesktop(page,true,undefined,true);await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
  await page.getByRole('button',{name:'同步设置 测试邮箱 1',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'测试邮箱 1 · IMAP 同步设置'});
  await dialog.getByLabel('继续下载更早的历史邮件').check();await dialog.getByLabel('自动同步',{exact:true}).selectOption('300');
  await dialog.getByRole('button',{name:'保存设置',exact:true}).click();await expect(dialog.getByRole('alert')).toHaveText('设置暂时未保存');
  await expect(dialog.getByLabel('继续下载更早的历史邮件')).toBeChecked();await dialog.getByRole('button',{name:'保存设置',exact:true}).click();
  await page.getByRole('button',{name:'同步设置 测试邮箱 1',exact:true}).click();await expect(dialog.getByLabel('自动同步',{exact:true})).toHaveValue('300');
  await page.screenshot({path:info.outputPath('imap-options.png')});await dialog.getByRole('button',{name:'取消',exact:true}).click();
  await page.getByRole('button',{name:'同步设置 测试邮箱 2',exact:true}).click();await expect(page.getByRole('dialog',{name:'测试邮箱 2 · IMAP 同步设置'}).getByLabel('自动同步',{exact:true})).toHaveValue('0');
});

test('connection editing supports mandatory STARTTLS and reports SMTP as skipped for receive-only', async ({page},info) => {
  await mockDesktop(page,false,undefined,true);await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
  await page.getByRole('button',{name:'重新配置 测试邮箱 1',exact:true}).click();const dialog=page.getByRole('dialog',{name:'更新 IMAP 邮箱连接'});
  await dialog.getByLabel('IMAP 加密').selectOption('starttls');await expect(dialog.getByLabel('IMAP 端口',{exact:true})).toHaveValue('143');
  await expect(dialog.getByLabel('SMTP 服务器')).toBeDisabled();await dialog.getByLabel('密码',{exact:true}).fill('synthetic');
  await dialog.getByRole('button',{name:'测试连接',exact:true}).click();await expect(dialog.getByText('SMTP 未验证（仅收信）')).toBeVisible();
  await expect(dialog.getByText(/SMTP连接成功/)).toHaveCount(0);await page.screenshot({path:info.outputPath('imap-connection.png')});
});

test('OAuth configuration waits for actual authorization and can be cancelled without a saved success', async ({page},info) => {
  await mockDesktop(page);await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
  await page.getByRole('button',{name:'添加账号',exact:true}).click();
  const wizard=page.getByRole('dialog',{name:'添加邮箱账号'});
  await wizard.getByRole('button',{name:'添加 Gmail 账号',exact:true}).click();
  const gmail=page.getByRole('dialog',{name:'添加 Gmail 账号'});
  await expect(gmail.getByLabel('Client ID',{exact:true})).toHaveCount(0);
  await gmail.getByLabel('账号名称',{exact:true}).fill('OAuth 测试');await gmail.getByLabel('邮箱地址',{exact:true}).fill('oauth@gmail.test');
  await gmail.getByRole('button',{name:'在浏览器中授权',exact:true}).click();
  await expect(gmail.getByText('等待系统浏览器授权…')).toBeVisible();await expect(gmail.getByRole('button',{name:'刷新账号列表',exact:true})).toHaveCount(0);
  await page.screenshot({path:info.outputPath('oauth-waiting.png')});
  await gmail.getByRole('button',{name:'取消授权',exact:true}).click();
  await expect(gmail.getByText('授权已取消，账号未保存。')).toBeVisible();
  await expect(gmail.getByRole('button',{name:'重新授权',exact:true})).toBeVisible();
  await expect(gmail.getByLabel('邮箱地址',{exact:true})).toHaveValue('oauth@gmail.test');
});

test('provider picker disables unregistered platforms and prefills password IMAP presets', async ({page},info) => {
  await mockDesktop(page);await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
  await page.getByRole('button',{name:'添加账号',exact:true}).click();
  const wizard=page.getByRole('dialog',{name:'添加邮箱账号'});
  await expect(wizard.getByRole('button',{name:'Outlook / Microsoft 365，即将支持',exact:true})).toBeDisabled();
  await wizard.getByRole('button',{name:'添加 QQ 邮箱 账号',exact:true}).click();
  const qq=page.getByRole('dialog',{name:'添加 QQ 邮箱 账号'});
  await expect(qq.getByLabel('IMAP 服务器')).toHaveValue('imap.qq.com');
  await expect(qq.getByLabel('SMTP 服务器')).toHaveValue('smtp.qq.com');
  await expect(qq.getByLabel('授权码',{exact:true})).toBeVisible();
  await qq.getByLabel('账号名称',{exact:true}).fill('QQ 测试');await qq.getByLabel('邮箱地址',{exact:true}).fill('user@qq.test');
  await expect(qq.getByLabel('登录用户')).toHaveValue('user@qq.test');
  await qq.getByRole('button',{name:'返回',exact:true}).click();
  // POP3 卡片可手动填服务器（无预设地址）。
  await wizard.getByRole('button',{name:'添加 POP3 邮箱 账号',exact:true}).click();
  const pop3=page.getByRole('dialog',{name:'添加 POP3 邮箱 账号'});
  await expect(pop3.getByLabel('POP3 服务器')).toBeVisible();
  await expect(pop3.getByLabel('密码',{exact:true})).toBeVisible();
  await pop3.getByRole('button',{name:'返回',exact:true}).click();
  await expect(wizard.getByRole('button',{name:'添加 Gmail 账号',exact:true})).toBeVisible();
  await page.screenshot({path:info.outputPath('provider-picker.png')});
});

test('attachment cancellation and missing raw cache never report success; later retry can save',async ({page})=>{
  await mockDesktop(page,true,undefined,true);await page.goto('/');await page.getByRole('button',{name:'阅读邮件：离线状态测试',exact:true}).click();
  const save=page.getByRole('button',{name:'保存附件 report.txt',exact:true});await save.click();await expect(page.getByText('已保存附件：report.txt')).toHaveCount(0);
  await save.click();await expect(page.getByRole('alert').filter({hasText:'原文缓存已被清理，请重新下载'})).toBeVisible();await expect(page.getByText('已保存附件：report.txt')).toHaveCount(0);
  await save.click();await expect(page.getByText('已保存附件：report.txt',{exact:true})).toBeVisible();
});

test('server folders follow the chosen account and server drafts create a separate local copy',async ({page})=>{
  await mockDesktop(page,true,undefined,true);await page.goto('/');await page.getByRole('button',{name:'测试邮箱 1',exact:true}).click();
  await page.getByRole('navigation',{name:'服务器文件夹',exact:true}).getByRole('button',{name:'项目邮件',exact:true}).click();await expect(page.getByRole('button',{name:'阅读邮件：离线状态测试',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'测试邮箱 2',exact:true}).click();await expect(page.getByRole('navigation',{name:'服务器文件夹',exact:true})).toHaveCount(0);
  await page.getByRole('button',{name:'测试邮箱 1',exact:true}).click();await page.getByRole('navigation',{name:'邮件文件夹',exact:true}).getByRole('button',{name:/草稿/}).click();
  await page.getByRole('button',{name:'阅读邮件：服务器草稿测试',exact:true}).click();await page.getByRole('button',{name:'复制为本地草稿',exact:true}).click();
  await page.getByRole('textbox',{name:'邮件正文'}).fill('新的本地副本');await expect(page.getByRole('status').filter({hasText:'草稿已自动保存'})).toBeVisible();
  await page.getByRole('button',{name:'关闭撰写并返回列表',exact:true}).click();
  await expect(page.getByRole('button',{name:'阅读邮件：服务器草稿测试',exact:true})).toHaveCount(2);
});


test('titlebar blank and brand drag while double-click maximizes, controls stay independent',async ({page})=>{
  await mockDesktop(page);await page.goto('/');await expect(page.getByRole('region',{name:'邮件列表'})).toBeVisible();
  const calls=()=>page.evaluate(()=>(window as unknown as {__syncTest:{windowCalls:()=>string[]}}).__syncTest.windowCalls());
  await page.getByRole('searchbox',{name:'搜索邮件'}).click();await expect.poll(calls).toEqual([]);
  await page.getByRole('banner').getByRole('button',{name:'设置',exact:true}).click();await page.keyboard.press('Escape');await expect.poll(calls).toEqual([]);
  await page.locator('.brand strong').dispatchEvent('mousedown',{button:0,buttons:1,detail:1});await expect.poll(calls).toEqual(['plugin:window|start_dragging']);
  await page.getByRole('banner').dispatchEvent('mousedown',{button:0,buttons:1,detail:2});await expect.poll(calls).toEqual(['plugin:window|start_dragging','plugin:window|toggle_maximize']);
  await page.getByRole('banner').dispatchEvent('mousedown',{button:2,buttons:2,detail:1});await expect.poll(calls).toEqual(['plugin:window|start_dragging','plugin:window|toggle_maximize']);
  await page.getByRole('button',{name:'最大化或还原',exact:true}).click();await expect.poll(calls).toEqual(['plugin:window|start_dragging','plugin:window|toggle_maximize','plugin:window|toggle_maximize']);
});

test('rich mail renders in a sandboxed iframe, remote images stay gated and links open externally', async ({page},info) => {
  await mockDesktop(page,true,undefined,true);await page.goto('/');await page.getByRole('button',{name:'阅读邮件：离线状态测试',exact:true}).click();
  // Playwright 无法进入 opaque-origin 的 srcdoc 框架，断言落在 srcdoc 属性与父页通路上。
  const frame=page.locator('iframe.mail-html-frame');
  await expect(frame).toHaveAttribute('sandbox','allow-scripts');
  await expect(frame).toHaveAttribute('srcdoc',/富文本正文（远程图片被拦截）/);
  await expect(page.getByText('已拦截远程图片，防止发件人追踪读取行为。')).toBeVisible();
  await page.screenshot({path:info.outputPath('mail-html.png')});
  await page.getByRole('button',{name:'加载远程图片',exact:true}).click();
  await expect(frame).toHaveAttribute('srcdoc',/富文本正文（远程图片已允许）/);
  await expect(frame).toHaveAttribute('srcdoc',/img-src https: data:/);
  await expect(page.getByText('已拦截远程图片，防止发件人追踪读取行为。')).toHaveCount(0);
  // 邮件内链接点击在真实浏览器里由桥接脚本转发；这里验证父页通路到 open_external。
  await page.evaluate(() => window.dispatchEvent(new MessageEvent('message',{data:{type:'auumail-mail-link',href:'https://example.test/receipt'}})));
  await page.getByRole('button',{name:'阅读邮件：纯文本链接测试',exact:true}).click();
  await expect(page.locator('.mail-link',{hasText:'https://example.com/help'})).toBeVisible();
  await page.locator('.mail-link',{hasText:'https://example.com/help'}).click();
  const calls=await page.evaluate(() => (window as unknown as { __syncTest: { openCalls: () => string[]; htmlRequests: () => Array<{ id: string; allow: boolean }> } }).__syncTest.openCalls());
  const requests=await page.evaluate(() => (window as unknown as { __syncTest: { htmlRequests: () => Array<{ id: string; allow: boolean }> } }).__syncTest.htmlRequests());
  await expect(calls).toEqual(['https://example.test/receipt','https://example.com/help']);
  // 开发模式 StrictMode 会对首次选择双触发 effect；语义断言：默认拦截请求在前、允许请求存在、
  // 且从未为演示账号的纯文本邮件请求富文本。
  await expect(requests[0]).toEqual({id:'real-message',allow:false});
  await expect(requests.some(request => request.allow)).toBe(true);
  await expect(requests.every(request => request.id === 'real-message')).toBe(true);
  // 操作区在正文上方：回复 / 回复全部 / 转发创建本地草稿（发送尚未接入）。
  await page.getByRole('button',{name:'转发',exact:true}).click();
  const composePane=page.locator('.compose-pane');
  await expect(composePane.getByLabel('主题')).toHaveValue(/^转发：/);
  await expect(composePane.getByRole('textbox',{name:'邮件正文'})).toContainText('引用的原文 HTML');
  await page.getByRole('button',{name:'关闭撰写并返回列表',exact:true}).click();
  // 关闭即自动保存草稿，撰写面直接收起（不再出现确认条）。
  await expect(page.locator('.compose-pane')).toHaveCount(0);
  await page.getByRole('button',{name:'阅读邮件：离线状态测试',exact:true}).click();
  await page.getByRole('button',{name:'回复全部',exact:true}).click();
  const replyAll=page.locator('.compose-pane');
  await expect(replyAll.getByLabel('主题')).toHaveValue('Re: 离线状态测试');
  await expect(replyAll.getByLabel('收件人')).toHaveValue(/zhixia@example\.test/);
  await expect(replyAll.getByLabel('收件人')).toHaveValue(/alex@example\.test/);
  await expect(replyAll.getByLabel('收件人')).not.toHaveValue(/test1@mail\.example/);
});

test('send is capability-gated, the outbox queue surfaces state, and unknown offers explicit exits', async ({page},info) => {
  await mockDesktop(page,true,undefined,true);await page.goto('/');
  // 演示账号草稿：发送按钮禁用并说明原因。
  await page.getByRole('button',{name:'阅读邮件：让下一版界面，多一点呼吸感',exact:true}).click();
  await page.getByRole('button',{name:'回复邮件',exact:true}).click();
  const demoCompose=page.locator('.compose-pane');
  await expect(demoCompose.getByRole('button',{name:'发送',exact:true})).toBeDisabled();
  await demoCompose.getByRole('button',{name:'关闭撰写并返回列表',exact:true}).click();
  await expect(demoCompose).toHaveCount(0);
  // 真实账号：回复 → 发送 → 队列徽标。
  await page.getByRole('button',{name:'阅读邮件：离线状态测试',exact:true}).click();
  await page.getByRole('button',{name:'回复邮件',exact:true}).click();
  const compose=page.locator('.compose-pane');
  await compose.getByRole('textbox',{name:'邮件正文'}).fill('发送测试');
  await compose.getByRole('button',{name:'发送',exact:true}).click();
  // 登记成功后撰写框自动关闭，结果以 toast 反馈。
  await expect(compose).toHaveCount(0);
  await expect(page.locator('.toast')).toContainText('已进入发送队列');
  await page.screenshot({path:info.outputPath('send-queued.png')});
  await page.getByRole('banner').getByRole('button',{name:'设置',exact:true}).click();
  await expect(page.getByText('发送排队中')).toBeVisible();
  // 结果不明：两个明确出口，不自动重发。
  await page.evaluate(() => (window as unknown as { __syncTest: { outboxState: (state: string) => void } }).__syncTest.outboxState('unknown'));
  await expect(page.getByText(/结果不明（已尝试 1 次）/)).toBeVisible({timeout:7000});
  await page.getByRole('tabpanel',{name:'邮箱账号'}).getByRole('button',{name:'仅保留本机',exact:true}).click();
  await expect(page.getByText(/发送失败：已按仅保留本机处理/)).toBeVisible({timeout:7000});
  const actions=await page.evaluate(() => (window as unknown as { __syncTest: { outboxActions: () => Array<{ id: string; action: string }> } }).__syncTest.outboxActions());
  await expect(actions).toEqual([{id:'ob-1',action:'keep-local'}]);
});

test('sync lives in the workspace chrome with header sync-all, toolbar per-account sync and completion toast', async ({page}) => {
  await mockDesktop(page,true,undefined,true);await page.goto('/');
  // 顶栏同步全部：排队 toast + 按钮转圈禁用。
  await page.getByRole('banner').getByRole('button',{name:'同步全部账号',exact:true}).click();
  await expect(page.locator('.toast')).toContainText('已排队同步全部账号');
  await expect(page.getByRole('banner').getByRole('button',{name:'同步全部账号',exact:true})).toBeDisabled();
  // 邮件列表工具栏：全部账号视图 → 同步中…禁用（aria-label 为同步全部账号，断言内部文字）。
  await expect(page.locator('.list-actions .account-sync')).toBeDisabled();
  await expect(page.locator('.list-actions .account-sync')).toContainText('同步中…');
  // 指定账号视图：同步按钮跟随任务状态。
  await page.getByRole('button',{name:'测试邮箱 1',exact:true}).click();
  await expect(page.locator('.list-actions .account-sync')).toContainText('同步中…');
  // 完成后 toast 反馈（设置关闭也可见）。
  await page.evaluate(() => (window as unknown as { __syncTest: { finish: (id: string, failure: boolean) => void } }).__syncTest.finish('real-1', false));
  await expect(page.locator('.toast')).toContainText('同步完成');
});
