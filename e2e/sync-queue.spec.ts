import { expect, test, type Page } from '@playwright/test';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { MailAccount, MailDraft, MailMessage, MessagePatch } from '../src/domain/mail';
import type { SyncJob } from '../src/domain/sync';

/** Test-only IPC harness. Validates UI behavior, not a real server or the Rust queue. */
async function mockDesktop(page: Page, messageChanges = false, recoveryMode?: 'legacy' | 'unknown' | 'queued' | 'paged' | 'fingerprint' | 'unanchored' | 'segmented', completion = false) {
  const accounts: MailAccount[] = [1, 2, 3].map(index => ({ ...demoWorkspace.accounts[0], id: `real-${index}`, name: `测试邮箱 ${index}`, email: `test${index}@mail.example`, provider: 'imap', status: 'offline' }));
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
    }
    const imapSettings = new Map<string,{syncAllFolders:boolean;historyEnabled:boolean;autoSyncSeconds:number}>();
    let settingFails = completion; let exportCalls = 0; let oauthState = 'waiting';
    const recoveryActions = new Map<string, string>();
    let nextRecovery: 'located' | 'source-present' | 'unresolved' | 'scanning' = 'located';
    let segmentPhase: 'collecting' | 'validating' | 'located' | 'unresolved' = 'collecting';
    let revision = 0; let sequence = 0;
    const jobs: SyncJob[] = []; const drafts = new Map<string, MailDraft>();
    const snapshot = () => structuredClone({ revision, jobs });
    function project(draft: MailDraft): MailMessage {
      const account = workspace.accounts.find(account => account.id === draft.accountId)!;
      return { id: draft.id, accountId: draft.accountId, threadId: draft.replyToId ?? draft.id, mailbox: 'drafts', from: { name: account.name, email: account.email }, to: draft.to ? [draft.to] : [], subject: draft.subject || '未命名草稿', preview: draft.bodyText.slice(0, 120), bodyText: draft.bodyText, receivedAt: draft.updatedAt, unread: false, starred: false, labelIds: [], draft: { ...draft } };
    }
    const invoke = async (command: string, input: Record<string, unknown> = {}) => {
      switch (command) {
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
  await expect(page.getByRole('status').filter({ hasText: '草稿已保存' })).toBeVisible();
  await page.getByRole('button', { name: '关闭对话框', exact: true }).click();
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
  await expect(page.getByText(/同步失败：测试连接失败/)).toBeVisible();
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
  await mockDesktop(page);await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();await page.getByRole('button',{name:'添加 IMAP 账号',exact:true}).click();
  await page.getByLabel('账号名称',{exact:true}).fill('OAuth 测试');await page.getByLabel('邮箱地址',{exact:true}).fill('oauth@example.test');
  await page.getByRole('button',{name:'使用 Gmail / Microsoft 授权',exact:true}).click();const dialog=page.getByRole('dialog',{name:'使用 OAuth 添加 IMAP 账号'});
  await dialog.getByLabel('Client ID',{exact:true}).fill('synthetic-public-id');await dialog.getByRole('button',{name:'在浏览器中授权',exact:true}).click();
  await expect(dialog.getByText('等待系统浏览器授权…')).toBeVisible();await expect(dialog.getByRole('button',{name:'刷新账号列表',exact:true})).toHaveCount(0);
  await page.screenshot({path:info.outputPath('oauth-waiting.png')});await dialog.getByRole('button',{name:'取消授权',exact:true}).click();await expect(dialog).toHaveCount(0);
  await expect(page.getByRole('dialog',{name:'添加 IMAP 邮箱账号'}).getByLabel('邮箱地址',{exact:true})).toHaveValue('oauth@example.test');
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
  await page.getByRole('textbox',{name:'邮件正文'}).fill('新的本地副本');await expect(page.getByRole('status').filter({hasText:'草稿已保存'})).toBeVisible();
  await page.getByRole('dialog').getByRole('button',{name:'关闭对话框',exact:true}).click();
  await expect(page.getByRole('button',{name:'阅读邮件：服务器草稿测试',exact:true})).toHaveCount(2);
});
