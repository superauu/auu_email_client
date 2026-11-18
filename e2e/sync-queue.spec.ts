import { expect, test, type Page } from '@playwright/test';
import { demoWorkspace } from '../src/services/demo/fixtures';
import type { MailAccount, MailDraft, MailFolder, MailMessage, MessagePatch } from '../src/domain/mail';
import type { SyncJob } from '../src/domain/sync';
import type { OutboxEntry } from '../src/domain/outbox';

/** Test-only IPC harness. Validates UI behavior, not a real server or the Rust queue. */
async function mockDesktop(page: Page, messageChanges = false, recoveryMode?: 'legacy' | 'unknown' | 'queued' | 'paged' | 'fingerprint' | 'unanchored' | 'segmented', completion = false, navigation = false, realAccounts = true, options: { pop3?: boolean; outbox?: OutboxEntry[]; huge?: number; threads?: boolean } = {}) {
  const accounts: MailAccount[] = (realAccounts ? [1, 2, 3] : []).map(index => ({ ...demoWorkspace.accounts[0], id: `real-${index}`, name: `测试邮箱 ${index}`, email: `test${index}@mail.example`, provider: 'imap', status: 'offline', capabilities: { ...demoWorkspace.accounts[0].capabilities, send: true } }));
  if (options.pop3) accounts[2] = { ...accounts[2], provider: 'pop3' };
  await page.addInitScript(({ fixture, accounts, messageChanges, recoveryMode, completion, navigation, options }) => {
    const workspace = { ...fixture, accounts: [...fixture.accounts, ...accounts] };
    if (messageChanges || completion) workspace.messages.unshift({ ...fixture.messages[0], id: 'real-message', accountId: 'real-1', labelIds: [], unread: false, subject: '离线状态测试', stateRevision: 1, changes: [{ field: 'starred', state: 'conflict', error: 'UIDVALIDITY 已改变，请核对服务器。' }], cc: ['copy@mail.example'], replyTo: ['replies@mail.example'] });
    if (options.huge) {
      const base = workspace.messages[0];
      for (let index = 0; index < options.huge; index++) {
        workspace.messages.push({ ...base, id: `huge-${index}`, accountId: 'real-1', labelIds: [], unread: index % 7 === 0, subject: `大数据量邮件 ${index + 1}`, bodyText: `批量测试正文 ${index + 1}`, receivedAt: new Date(Date.UTC(2026, 0, 1) + index * 60000).toISOString(), changes: [], attachments: [] });
      }
    }
    if (options.threads) {
      const base = workspace.messages[0];
      const inject = (id: string, subject: string, at: string, unread = false): MailMessage => { const item = { ...base, id, accountId: 'real-1', labelIds: [], unread, subject, bodyText: `会话正文 ${id}`, receivedAt: at, changes: [], attachments: [] }; workspace.messages.push(item); return item; };
      inject('thread-1', '季度评审报告', '2026-03-01T00:00:00Z', true);
      inject('thread-2', 'Re: 季度评审报告', '2026-03-02T00:00:00Z');
      inject('thread-3', '转发：季度评审报告', '2026-03-03T00:00:00Z', true);
      inject('thread-solo', '独立通知', '2026-03-04T00:00:00Z');
      // 引用链会话：回复改了主题，References 仍连回根——对话模式下应与根同组。
      const chained = inject('chain-root', '立项讨论（原主题）', '2026-03-05T00:00:00Z');
      inject('chain-reply', '改主题后的跟进', '2026-03-06T00:00:00Z');
      const chainReply = workspace.messages.at(-1)!;
      chainReply.headerMessageId = 'chain-reply@mail';
      chainReply.headerReferences = 'chain-root@mail';
      chained.headerMessageId = 'chain-root@mail';
    }
    if (recoveryMode) { const message = workspace.messages.find(message => message.id === 'real-message')!; message.recovery = { state: recoveryMode === 'queued' ? 'queued' : 'available', detail: '旧缓存或未知归档需要远端核对。', location: null, evidence: null }; if (recoveryMode === 'unknown') { message.mailbox = 'archive'; message.changes = [{ field: 'archive', state: 'unknown', error: '归档结果待核对，不会自动重试。' }]; } }
    if (recoveryMode === 'fingerprint' || recoveryMode === 'unanchored') { const message = workspace.messages.find(message => message.id === 'real-message')!; message.recovery!.fingerprint = { eligible: recoveryMode === 'fingerprint', reason: recoveryMode === 'fingerprint' ? '可信原定位仍在，可安全核对原文。' : '缺少可信原定位，不能升级为历史原文指纹。' }; }
    if (completion) {
      const message = workspace.messages.find(message => message.id === 'real-message')!;
      message.attachments = [{index:0,name:'report.txt',size:1024,mediaType:'text/plain'}];
      message.folderIds=['project-1'];
      workspace.messages.push({...message,id:'server-draft',mailbox:'drafts',subject:'服务器草稿测试',changes:[],attachments:[]});
      workspace.messages.push({...fixture.messages[1], id:'plain-link', accountId: 'real-1', labelIds: [], subject:'纯文本链接测试', bodyText:'请联系 help@anoma.ly 或访问 https://example.com/help。', unread:false, changes:[], attachments:[]});
    }
    const folderList: MailFolder[] = completion ? [{id:'project-1',accountId:'real-1',name:'项目邮件',kind:'other',available:true}] : [];
    if (navigation) {
      folderList.push(...[
        { id: 'inbox-1', name: 'INBOX', kind: 'inbox' },
        { id: 'drafts-1', name: 'Drafts', kind: 'drafts' },
        { id: 'trash-1', name: 'Deleted Messages', kind: 'trash' },
        { id: 'long-1', name: 'Projects/2026/这是一个很长很长的项目归档目录名称，用于检查窄窗口', kind: 'other' },
      ].map(folder => ({ ...folder, accountId: 'real-1', available: true })));
      workspace.messages.find(message => message.id === 'real-message')!.folderIds = ['inbox-1', 'project-1'];
      workspace.messages.find(message => message.id === 'server-draft')!.folderIds = ['drafts-1'];
    }
    const imapSettings = new Map<string,{syncAllFolders:boolean;historyEnabled:boolean;autoSyncSeconds:number}>();
    let settingFails = completion; let exportCalls = 0; let oauthState = 'waiting'; let pickFailure = false;
    const htmlRequests: Array<{ id: string; allow: boolean }> = []; const openCalls: string[] = [];
    const outboxActions: Array<{ id: string; action: string }> = [];
    let outboxRows = options.outbox ?? null;
    let outboxResolved: string | null = null;
    let sentDraftId = 'sent-draft';
    let outboxFails = false;
    const accountInputs: Array<{ command: string; provider?: string }> = [];
    let outboxState: 'queued' | 'sending' | 'unknown' | 'failed' | 'sent' | null = null;
    const outboxEntry = (): OutboxEntry | null => outboxState === null ? null : { id: 'ob-1', draftId: sentDraftId, accountId: 'real-1', messageId: '<m@auumail.local>', state: outboxState, attempts: 1, lastError: outboxState === 'unknown' ? '邮件已提交给服务器，但未能确认投递结果' : outboxState === 'failed' ? '已按仅保留本机处理；服务器是否投递未被确认，不会自动重试' : null, queuedAt: new Date().toISOString(), updatedAt: new Date().toISOString(), resolvedBy: outboxResolved };
    const recoveryActions = new Map<string, string>();
    let nextRecovery: 'located' | 'source-present' | 'unresolved' | 'scanning' = 'located';
    let segmentPhase: 'collecting' | 'validating' | 'located' | 'unresolved' = 'collecting';
    let revision = 0; let sequence = 0;
    let saveGate: Promise<void> | null = null;
    let releaseSave = () => {};
    const jobs: SyncJob[] = []; const drafts = new Map<string, MailDraft>();
    for (const entry of outboxRows ?? []) drafts.set(entry.draftId, { id: entry.draftId, accountId: entry.accountId, to: 'recipient@example.test', cc: 'copy@example.test', bcc: 'private@example.test', subject: `任务 ${entry.id}`, bodyText: `保留的正文 ${entry.id}`, bodyHtml: `<p>保留的正文 ${entry.id}</p>`, revision: 3, updatedAt: entry.updatedAt, attachments: [{ id: 'att-test', name: '测试附件.txt', size: 3, mediaType: 'text/plain', sha256: 'a'.repeat(64) }] });
    const windowCalls: string[] = [];
    const workspaceCalls: Array<{ command: string; accountId?: string | null }> = [];
    const snapshot = () => structuredClone({ revision, jobs });
    function project(draft: MailDraft): MailMessage {
      const account = workspace.accounts.find(account => account.id === draft.accountId)!;
      return { id: draft.id, accountId: draft.accountId, threadId: draft.replyToId ?? draft.id, mailbox: 'drafts', from: { name: account.name, email: account.email }, to: draft.to ? [draft.to] : [], subject: draft.subject || '未命名草稿', preview: draft.bodyText.slice(0, 120), bodyText: draft.bodyText, receivedAt: draft.updatedAt, unread: false, starred: false, labelIds: [], draft: { ...draft }, ...(draft.cc ? { cc: draft.cc.split(',').map(address => address.trim()).filter(Boolean) } : {}) };
    }
    const invoke = async (command: string, input: Record<string, unknown> = {}) => {
      switch (command) {
        case 'plugin:window|start_dragging': case 'plugin:window|toggle_maximize': case 'plugin:window|minimize': windowCalls.push(command);return;
        case 'get_app_info': return { version: '0.1.0', dataDirectory: 'test-only', skinDirectory: 'test-only/skins', databaseReady: true };
        case 'list_user_skins': return { skins: [], warnings: [] };
        case 'mail_seed_workspace': case 'mail_import_legacy_drafts': throw new Error('真实工作区禁止初始化或导入演示数据');
        case 'plugin:event|unlisten': return;
        case 'mail_load_workspace': return { accounts: workspace.accounts, labels: workspace.labels, events: workspace.events, folders: folderList, syncStates: [] };
        // 分页镜像 Rust page_messages：意图投影已由 stub 的 mailbox 变更体现，这里按视图过滤 + 偏移游标分页。
        case 'mail_page_messages': {
          const query = input.query as { accountId?: string | null; mailbox?: string | null; folderId?: string | null; labelId?: string | null; search?: string | null; cursor?: string | null; limit?: number | null };
          workspaceCalls.push({ command, accountId: query.accountId ?? null });
          const realIds = new Set(workspace.accounts.filter(account => account.status !== 'demo').map(account => account.id));
          let items = workspace.messages.filter(m => realIds.has(m.accountId) && (!query.accountId || m.accountId === query.accountId));
          const folderId = query.folderId;
          const labelId = query.labelId;
          if (folderId) items = items.filter(m => (m.folderIds ?? []).includes(folderId));
          else if (query.mailbox === 'starred') items = items.filter(m => m.starred && m.mailbox !== 'trash');
          else if (query.mailbox) items = items.filter(m => m.mailbox === query.mailbox);
          if (labelId) items = items.filter(m => (m.labelIds ?? []).includes(labelId) && m.mailbox !== 'trash');
          if (query.search) { const needle = query.search.toLowerCase(); items = items.filter(m => [m.subject, m.from?.name, m.from?.email, m.bodyText].some(v => String(v ?? '').toLowerCase().includes(needle))); }
          items.sort((a, b) => b.receivedAt.localeCompare(a.receivedAt) || (a.id < b.id ? 1 : a.id > b.id ? -1 : 0));
          const limit = query.limit ?? 100;
          const start = query.cursor ? Number(query.cursor) : 0;
          return { items: structuredClone(items.slice(start, start + limit)), nextCursor: start + limit < items.length ? String(start + limit) : null };
        }
        // 侧栏计数镜像 Rust folder_counts：kind 投影 + 星标（排除废纸篓）+ 本机草稿 + 物理目录成员。
        case 'mail_folder_counts': {
          const counts: Array<{ accountId: string; view: string; total: number; unread: number }> = [];
          for (const account of workspace.accounts.filter(account => account.status !== 'demo')) {
            const mine = workspace.messages.filter(m => m.accountId === account.id && !m.draft);
            for (const kind of ['inbox', 'sent', 'drafts', 'archive', 'trash', 'other']) {
              const rows = mine.filter(m => m.mailbox === kind);
              if (rows.length) counts.push({ accountId: account.id, view: `kind:${kind}`, total: rows.length, unread: rows.filter(m => m.unread).length });
            }
            const starred = mine.filter(m => m.starred && m.mailbox !== 'trash');
            if (starred.length) counts.push({ accountId: account.id, view: 'starred', total: starred.length, unread: starred.filter(m => m.unread).length });
            const localDrafts = [...drafts.values()].filter(d => d.accountId === account.id);
            if (localDrafts.length) counts.push({ accountId: account.id, view: 'drafts-local', total: localDrafts.length, unread: 0 });
            for (const folder of folderList.filter(f => f.accountId === account.id)) {
              const rows = workspace.messages.filter(m => m.accountId === account.id && (m.folderIds ?? []).includes(folder.id));
              if (rows.length) counts.push({ accountId: account.id, view: `folder:${folder.id}`, total: rows.length, unread: rows.filter(m => m.unread).length });
            }
          }
          return counts;
        }
        // 撰写收件人建议镜像 Rust contact_suggestions：最近邮件发件人 + 收件人去重。
        case 'mail_contact_suggestions': {
          const contacts = new Map<string, { name: string; email: string }>();
          for (const m of workspace.messages.filter(m => m.accountId === input.accountId && !m.draft)) {
            for (const contact of [m.from, ...(m.to ?? []).map((email: string) => ({ name: '', email })), ...((m.cc ?? m.replyTo ?? []) as string[]).map(email => ({ name: '', email }))]) {
              const email = contact?.email;
              if (email && email.includes('@') && !contacts.has(email.toLowerCase())) contacts.set(email.toLowerCase(), { name: contact.name ?? '', email });
            }
          }
          return [...contacts.values()];
        }
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
          if (patch.delete) message.mailbox = 'trash';
          if (patch.moveTo) { const target = folderList.find(folder => folder.id === patch.moveTo); if (target) message.mailbox = target.kind as typeof message.mailbox; }
          if (message.accountId.startsWith('real-')) {
            for (const field of ['unread', 'starred', 'archive', 'delete', 'move'] as const) if ((patch as Record<string, unknown>)[field] !== undefined) message.changes = [...(message.changes ?? []).filter(change => change.field !== field), { field, state: 'queued', error: null }];
          }
          message.stateRevision = (message.stateRevision ?? 0) + 1;
          return structuredClone(message);
        }
        // 目录 CRUD：镜像 imap_folders —— 标准目录只读，服务器确认后更新本地缓存并返回最新列表。
        case 'mail_create_folder': {
          const name = String(input.name).trim();
          if (!name || folderList.some(folder => folder.accountId === input.accountId && folder.name === name)) throw new Error('同名目录已存在');
          folderList.push({ id: `${String(input.accountId)}:folder:${name}`, accountId: String(input.accountId), name, kind: 'other' as const, available: true });
          return structuredClone(folderList.filter(folder => folder.accountId === input.accountId));
        }
        case 'mail_rename_folder': {
          const folder = folderList.find(folder => folder.id === input.folderId && folder.accountId === input.accountId);
          if (!folder || folder.kind !== 'other') throw new Error('目录不存在或不可修改');
          folder.name = String(input.newName).trim();
          folder.id = `${input.accountId}:folder:${folder.name}`;
          return structuredClone(folderList.filter(f => f.accountId === input.accountId));
        }
        case 'mail_delete_folder': {
          const index = folderList.findIndex(folder => folder.id === input.folderId && folder.accountId === input.accountId);
          if (index < 0 || folderList[index].kind !== 'other') throw new Error('目录不存在或不可修改');
          folderList.splice(index, 1);
          return structuredClone(folderList.filter(folder => folder.accountId === input.accountId));
        }
        // 永久删除：镜像 message_changes::purge —— IMAP 账号排队（消息保留在 trash 投影），
        // 本测试桩统一返回 queued 行为（POP3 removed 分支由 Rust 单测覆盖）。
        case 'mail_purge_message': {
          const message = workspace.messages.find(message => message.id === input.messageId && message.accountId === input.accountId);
          if (!message || message.mailbox !== 'trash') throw new Error('邮件不在废纸篓，无法永久删除');
          message.changes = [...(message.changes ?? []).filter(change => change.field !== 'purge'), { field: 'purge', state: 'queued', error: null }];
          message.stateRevision = (message.stateRevision ?? 0) + 1;
          return { outcome: 'queued', message: structuredClone(message) };
        }
        case 'mail_keep_message_local': {
          const message = workspace.messages.find(message => message.id === input.messageId && message.accountId === input.accountId)!;
          message.changes = message.changes?.map(change => ({ ...change, state: 'local', error: null }));
          message.stateRevision = (message.stateRevision ?? 0) + 1; return structuredClone(message);
        }
        case 'mail_save_draft': {
          const draft = input.draft as MailDraft;
          if ((outboxRows ?? (outboxEntry() ? [outboxEntry()!] : [])).some(entry => entry.draftId === draft.id)) throw new Error('草稿已进入发送流程');
          if (saveGate) await saveGate;
          const saved = { ...draft, revision: (draft.revision ?? 0) + 1, updatedAt: new Date().toISOString() };
          drafts.set(saved.id, saved); return project(saved);
        }
        // 撰写附件：测试桩只回合成元数据（真实读取在 Rust）；可注入一次失败验证诚实报错。
        case 'mail_compose_pick_attachments': {
          if (pickFailure) { pickFailure = false; throw new Error('无法读取所选文件「损坏.bin」'); }
          const currentCount = (input.currentCount as number) ?? 0;
          if (currentCount >= 20) throw new Error('附件数量已达上限（每封邮件最多 20 个）');
          const picked = [{ id: `att-${++sequence}`, name: '季度报告.pdf', size: 2048, mediaType: 'application/pdf', sha256: 'a'.repeat(64) }, { id: `att-${++sequence}`, name: '这是一个非常非常非常长的中文附件名称用于检查截断与换行布局的处理方式最终版.docx', size: 8 * 1024 * 1024, mediaType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', sha256: 'b'.repeat(64) }];
          return picked.slice(0, 20 - currentCount);
        }
        case 'mail_imap_settings': return imapSettings.get(input.accountId as string) ?? {syncAllFolders:true,historyEnabled:false,autoSyncSeconds:0};
        case 'mail_save_imap_settings': if(settingFails){settingFails=false;throw new Error('设置暂时未保存');} imapSettings.set(input.accountId as string,input.settings as {syncAllFolders:boolean;historyEnabled:boolean;autoSyncSeconds:number});return;
        case 'mail_account_config': {const account=accounts.find(account=>account.id===input.id)!; return {id:account.id,provider:account.provider,name:account.name,email:account.email,imapHost:account.provider === 'pop3' ? 'pop.example.test' : 'imap.example.test',imapPort:account.provider === 'pop3' ? 995 : 993,imapSecurity:'tls',smtpHost:'',smtpPort:465,receiveOnly:true,authUser:account.email,avatar:'T',color:'teal'};}
        case 'mail_verify_account': accountInputs.push({ command, provider: (input.endpoints as { provider?: string }).provider }); return {imap:{ok:true,skipped:false,error:null,code:null},smtp:{ok:false,skipped:true,error:null,code:null},folderCount:2};
        case 'mail_save_account': accountInputs.push({ command, provider: (input.account as { provider?: string }).provider }); return 'real-3';
        case 'mail_oauth_start': oauthState='waiting';return 'flow-test';
        case 'mail_oauth_status': return {id:'flow-test',state:oauthState,accountId:null,error:null,folderCount:null};
        case 'mail_oauth_cancel': oauthState='cancelled';return {id:'flow-test',state:oauthState,accountId:null,error:null,folderCount:null};
        case 'mail_export_attachment': exportCalls++;if(exportCalls===2) throw new Error('原文缓存已被清理，请重新下载');return exportCalls>2;
        case 'mail_send_draft': {
          const draft = drafts.get(input.draftId as string);
          if (!draft) throw new Error('草稿不存在');
          if (draft.revision !== input.revision) throw new Error('草稿已被修改，请保存后重新发送');
          outboxState = 'queued';
          sentDraftId = draft.id;
          return outboxEntry()!;
        }
        case 'mail_outbox_list': return structuredClone(outboxRows ?? (outboxEntry() ? [outboxEntry()!] : []));
        case 'mail_message_html_fragment': return '<p>引用的原文 HTML</p>';
        case 'mail_outbox_cancel': case 'mail_outbox_return_draft': case 'mail_outbox_resolve': case 'mail_retry_outbox': {
          if (outboxFails) { outboxFails = false; throw new Error('发送状态已变化，请刷新'); }
          const entry = (outboxRows ?? (outboxEntry() ? [outboxEntry()!] : [])).find(entry => entry.id === input.id);
          if (!entry) throw new Error('任务不存在');
          const action = command === 'mail_outbox_resolve' ? input.action as string : command;
          outboxActions.push({ id: entry.id, action });
          if (command === 'mail_outbox_cancel' || command === 'mail_outbox_return_draft') {
            if (outboxRows) outboxRows = outboxRows.filter(item => item.id !== entry.id); else outboxState = null;
            return structuredClone(entry);
          }
          entry.state = input.action === 'keep-local' ? 'failed' : 'queued';
          entry.resolvedBy = input.action === 'keep-local' ? 'keep-local' : null;
          if (!outboxRows) { outboxState = entry.state; outboxResolved = entry.resolvedBy; }
          return structuredClone(entry);
        }
        case 'mail_message_html': {
          if (input.messageId === 'plain-link') throw new Error('原文缓存不可用，回退纯文本');
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
      workspaceCalls: () => structuredClone(workspaceCalls),
      holdDraftSave() { saveGate = new Promise<void>(resolve => { releaseSave = resolve; }); },
      releaseDraftSave() { saveGate = null; releaseSave(); },
      drafts: () => [...drafts.values()],
      messages: () => structuredClone(workspace.messages),
      failNextPick() { pickFailure = true; },
      windowCalls: () => [...windowCalls],
      openCalls: () => [...openCalls],
      htmlRequests: () => [...htmlRequests],
      outboxState(state: 'queued' | 'sending' | 'unknown' | 'failed' | 'sent' | null) { outboxState = state; },
      outboxActions: () => [...outboxActions],
      failNextOutbox() { outboxFails = true; },
      accountInputs: () => structuredClone(accountInputs),
      settings: () => Object.fromEntries(imapSettings),
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
  }, { fixture: demoWorkspace, accounts, messageChanges, recoveryMode, completion, navigation, options });
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
  await page.getByRole('button', { name: '测试邮箱 1', exact: true }).click(); await page.getByRole('navigation', { name: '测试邮箱 1的文件夹', exact: true }).getByRole('button', { name: '归档', exact: true }).click();
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
  await expect(page.getByRole('heading', { name: /^全部收件箱\s*0$/ })).toBeVisible();
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

test('desktop workspace isolates legacy demo accounts, mail, labels, events and pinned shortcuts', async ({ page }, info) => {
  await mockDesktop(page, true);
  await page.addInitScript(() => localStorage.setItem('auumail.ui.navigation.v1', JSON.stringify({ pinned: [{ accountId: 'work', mailbox: 'inbox' }], expanded: null })));
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /^全部收件箱\s*1$/ })).toBeVisible();
  await expect(page.locator('.account-select')).toHaveCount(3);
  await expect(page.locator('.app-statusbar')).toContainText('3 个邮箱账号');
  await expect(page.getByRole('button', { name: '个人邮箱', exact: true })).toHaveCount(0);
  await expect(page.getByRole('navigation', { name: '邮件标签', exact: true })).toHaveCount(0);
  await expect(page.getByText('已失效的常用入口', { exact: true })).toHaveCount(0);
  const search = page.getByRole('searchbox', { name: '搜索邮件' });
  await search.fill('让下一版界面');
  await expect(page.getByRole('button', { name: /^阅读邮件：/ })).toHaveCount(0);
  await search.fill('');
  await page.getByRole('button', { name: '事件与待办', exact: true }).click();
  await expect(page.locator('.event-card')).toHaveCount(0);
  await page.getByRole('button', { name: '邮件工作区', exact: true }).click();
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await expect(page.getByLabel('发件人', { exact: true }).locator('option')).toHaveCount(3);
  await expect(page.getByLabel('发件人', { exact: true })).toHaveValue('real-1');
  await page.getByRole('button', { name: '关闭撰写并返回列表', exact: true }).click();
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await expect(page.locator('.settings-accounts > div')).toHaveCount(3);
  const reads = await page.evaluate(() => (window as unknown as { __syncTest: { workspaceCalls: () => Array<{ accountId?: string | null }> } }).__syncTest.workspaceCalls());
  // 懒加载后消息按视图分页读取：读取范围只能是真实账号或全部账号视图（演示账号永不入内）。
  expect(reads.length).toBeGreaterThan(0);
  expect(reads.every(read => read.accountId === null || read.accountId?.startsWith('real-'))).toBe(true);
  await page.screenshot({ path: info.outputPath('real-accounts-only.png') });
  await page.reload();
  await expect(page.getByRole('heading', { name: /^全部收件箱\s*1$/ })).toBeVisible();
  await expect(page.locator('.account-select')).toHaveCount(3);
});

test('desktop with only legacy demo rows offers account setup without a demo fallback', async ({ page }, info) => {
  await mockDesktop(page, false, undefined, false, false, false);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: '添加你的第一个邮箱', exact: true })).toBeVisible();
  await expect(page.locator('.account-select')).toHaveCount(0);
  await expect(page.locator('.mail-row')).toHaveCount(0);
  await expect(page.locator('.app-statusbar')).toContainText('0 个邮箱账号');
  await expect(page.getByRole('button', { name: '写邮件', exact: false })).toBeDisabled();
  await expect(page.getByRole('button', { name: '同步全部账号', exact: true })).toBeDisabled();
  await expect(page.locator('.demo-badge')).toHaveText('本地工作区');
  await page.screenshot({ path: info.outputPath('empty-real-workspace.png') });
  await page.getByRole('button', { name: '添加邮箱', exact: true }).click();
  await expect(page.getByRole('button', { name: '添加账号', exact: true })).toBeEnabled();
  await expect(page.locator('.settings-accounts > div')).toHaveCount(0);
});

test('account tree maps physical folders once, keeps server drafts separate and marks cached folder pins', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true); await page.goto('/');
  await expect(page.locator('.demo-accounts')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '个人邮箱', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '测试邮箱 1', exact: true }).click();
  const folders = page.getByRole('navigation', { name: '测试邮箱 1的文件夹', exact: true });
  await expect(page.getByRole('navigation', { name: '服务器文件夹', exact: true })).toHaveCount(0);
  await expect(folders.getByRole('button', { name: '收件箱', exact: true })).toHaveCount(1);
  await expect(folders.getByRole('button', { name: '废纸篓', exact: true })).toHaveCount(1);
  await expect(folders.getByRole('button', { name: 'Deleted Messages', exact: true })).toHaveCount(0);
  await expect(folders.getByRole('button', { name: '服务器草稿', exact: true })).toHaveCount(1);
  await expect(folders.getByRole('button', { name: '本机草稿', exact: true })).toHaveCount(1);
  await folders.getByRole('button', { name: '服务器草稿', exact: true }).click();
  await expect(page.getByRole('heading', { name: /^服务器草稿\s*1$/ })).toBeVisible();
  await folders.getByRole('button', { name: '项目邮件', exact: true }).click();
  await expect(page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true })).toBeVisible();
  await folders.getByRole('button', { name: '固定 测试邮箱 1 / 项目邮件', exact: true }).click();
  const common = page.getByRole('navigation', { name: '常用入口', exact: true });
  await expect(common.getByRole('button', { name: '项目邮件 · 测试邮箱 1', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '测试邮箱 2', exact: true }).click();
  await expect(page.getByRole('button', { name: '阅读邮件：离线状态测试', exact: true })).toHaveCount(0);
  await common.getByRole('button', { name: '项目邮件 · 测试邮箱 1', exact: true }).click();
  await expect(page.getByRole('heading', { name: /^项目邮件\s*1$/ })).toBeVisible();
  await page.setViewportSize({ width: 900, height: 700 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  expect(await page.locator('.sidebar-scroll').evaluate(element => element.scrollWidth <= element.clientWidth)).toBeTruthy();
  await page.screenshot({ path: info.outputPath('account-tree-real-folders.png') });
});

test('server folders follow the chosen account and server drafts create a separate local copy',async ({page})=>{
  await mockDesktop(page,true,undefined,true);await page.goto('/');await page.getByRole('button',{name:'测试邮箱 1',exact:true}).click();
  await page.getByRole('navigation',{name:'测试邮箱 1的文件夹',exact:true}).getByRole('button',{name:'项目邮件',exact:true}).click();await expect(page.getByRole('button',{name:'阅读邮件：离线状态测试',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'测试邮箱 2',exact:true}).click();await expect(page.getByRole('navigation',{name:'测试邮箱 2的文件夹',exact:true}).getByRole('button',{name:'项目邮件',exact:true})).toHaveCount(0);
  await page.getByRole('button',{name:'测试邮箱 1',exact:true}).click();await page.getByRole('navigation',{name:'测试邮箱 1的文件夹',exact:true}).getByRole('button',{name:'服务器草稿',exact:true}).click();
  await page.getByRole('button',{name:'阅读邮件：服务器草稿测试',exact:true}).click();await page.getByRole('button',{name:'复制为本地草稿',exact:true}).click();
  await page.getByRole('textbox',{name:'邮件正文'}).fill('新的本地副本');await expect(page.getByRole('status').filter({hasText:'草稿已自动保存'})).toBeVisible();
  await page.getByRole('button',{name:'关闭撰写并返回列表',exact:true}).click();
  await expect(page.getByRole('button',{name:'阅读邮件：服务器草稿测试',exact:true})).toHaveCount(1);
  await page.getByRole('navigation',{name:'测试邮箱 1的文件夹',exact:true}).getByRole('button',{name:'本机草稿',exact:true}).click();
  await expect(page.getByRole('button',{name:'阅读邮件：服务器草稿测试',exact:true})).toHaveCount(1);
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
  // 纯文本测试邮件在原文缓存不可用时回退，不返回富文本结果。
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
  await expect(replyAll.getByRole('button', { name: '从收件人移除 zhixia@example.test', exact: true })).toBeVisible();
  await expect(replyAll.getByRole('button', { name: '从收件人移除 alex@example.test', exact: true })).toBeVisible();
  // 回复全部把 Cc 一并纳入（抄送人收到回复）；本账号地址被剔除。
  await expect(replyAll.getByRole('button', { name: '从抄送移除 copy@mail.example', exact: true })).toBeVisible();
  await expect(replyAll.getByRole('button', { name: '从收件人移除 test1@mail.example', exact: true })).toHaveCount(0);
});

test('compose stays editable during autosave and sends bcc-only after empty-subject confirmation', async ({ page }) => {
  await mockDesktop(page); await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  await page.getByLabel('发件人', { exact: true }).selectOption('real-1');
  await page.evaluate(() => (window as unknown as { __syncTest: { holdDraftSave: () => void } }).__syncTest.holdDraftSave());
  const editor = page.getByRole('textbox', { name: '邮件正文' });
  await editor.fill('第一版内容');
  await expect(page.getByRole('status').filter({ hasText: '正在保存草稿' })).toBeVisible();
  await expect(editor).toBeEditable();
  await editor.fill('保存过程中继续输入的最新内容');
  await page.evaluate(() => (window as unknown as { __syncTest: { releaseDraftSave: () => void } }).__syncTest.releaseDraftSave());
  await expect(page.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('请至少填写一位');
  await page.getByRole('button', { name: '密送', exact: true }).click();
  await page.getByRole('combobox', { name: '密送', exact: true }).fill('private@example.test');
  await page.getByRole('button', { name: '发送', exact: true }).click();
  await expect(page.getByText('这封邮件还没有主题，仍然发送吗？')).toBeVisible();
  await page.getByRole('button', { name: '仍然发送', exact: true }).click();
  await expect(page.locator('.compose-pane')).toHaveCount(0);
  const drafts = await page.evaluate(() => (window as unknown as { __syncTest: { drafts: () => MailDraft[] } }).__syncTest.drafts());
  expect(drafts).toHaveLength(1);
  expect(drafts[0]).toMatchObject({ bodyText: '保存过程中继续输入的最新内容', bcc: 'private@example.test', subject: '', to: '', accountId: 'real-1' });
});

test('compose attachments: pick shows chips, saves round-trip and removal stays honest', async ({page}, info) => {
  await mockDesktop(page); await page.goto('/');
  await page.getByRole('button', { name: '写邮件', exact: false }).click();
  const compose = page.locator('.compose-pane');
  const list = compose.getByRole('list', { name: '附件列表' });
  await expect(list).toHaveCount(0);
  await compose.getByRole('button', { name: '添加附件', exact: true }).click();
  await expect(list).toBeVisible();
  await expect(list.locator('li')).toHaveCount(2);
  const pdf = compose.getByRole('button', { name: '移除附件 季度报告.pdf', exact: true });
  await expect(pdf).toBeVisible();
  // 长文件名截断在 chip 内，不撑破撰写布局。
  const long = list.locator('li').nth(1);
  await expect(long.locator('.compose-attachment-name')).toBeVisible();
  await expect(compose.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  const stored = await page.evaluate(() => (window as unknown as { __syncTest: { drafts: () => MailDraft[] } }).__syncTest.drafts());
  expect(stored[0].attachments).toHaveLength(2);
  await page.screenshot({ path: info.outputPath('compose-attachments.png') });
  // 移除：列表只剩一项，草稿自动保存后附件元数据同步移除。
  await pdf.click();
  await expect(list.locator('li')).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => (window as unknown as { __syncTest: { drafts: () => MailDraft[] } }).__syncTest.drafts()[0]?.attachments?.length ?? 0)).toBe(1);
  // 选取失败：诚实报错，既有附件保留。
  await page.evaluate(() => (window as unknown as { __syncTest: { failNextPick: () => void } }).__syncTest.failNextPick());
  await compose.getByRole('button', { name: '添加附件', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('无法读取所选文件');
  await expect(list.locator('li')).toHaveCount(1);
});

test('send is capability-gated, the outbox queue surfaces state, and unknown offers explicit exits', async ({page},info) => {
  await mockDesktop(page,true,undefined,true);await page.goto('/');
  // 真实账号：回复 → 发送 → 队列徽标。
  await page.getByRole('button',{name:'阅读邮件：离线状态测试',exact:true}).click();
  await page.getByRole('button',{name:'回复邮件',exact:true}).click();
  const compose=page.locator('.compose-pane');
  // 普通回复优先 Reply-To 头，而不是发件人地址。
  await expect(compose.getByRole('button',{name:'从收件人移除 replies@mail.example',exact:true})).toBeVisible();
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
  await expect(page.getByText('仅保留本机（投递结果未确认，不会再次发送）', { exact: true })).toBeVisible({timeout:7000});
  await expect(page.getByRole('region', { name: '发送队列', exact: true }).getByRole('button', { name: '重试发送', exact: true })).toHaveCount(0);
  await expect(page.getByRole('region', { name: '发送队列', exact: true }).getByRole('button', { name: '返回草稿修改', exact: true })).toHaveCount(0);
  const actions=await page.evaluate(() => (window as unknown as { __syncTest: { outboxActions: () => Array<{ id: string; action: string }> } }).__syncTest.outboxActions());
  await expect(actions).toEqual([{id:'ob-1',action:'keep-local'}]);
});

const queueEntry = (id: string, state: OutboxEntry['state'], accountId = 'real-1'): OutboxEntry => ({ id, draftId: `draft-${id}`, accountId, state, messageId: `<${id}@example.test>`, attempts: 1, lastError: state === 'failed' ? '服务器明确拒绝：测试错误 ' + '长错误信息'.repeat(40) : null, queuedAt: '2026-11-16T00:00:00Z', updatedAt: '2026-11-16T00:00:00Z', resolvedBy: null });

test('outbox exposes old unknown tasks, account filters, retry failures and queued cancellation', async ({ page }) => {
  await mockDesktop(page, false, undefined, false, false, true, { outbox: [queueEntry('sent-1', 'sent'), queueEntry('sent-2', 'sent'), queueEntry('sent-3', 'sent'), queueEntry('old-unknown', 'unknown'), queueEntry('rejected', 'failed', 'real-2'), queueEntry('sending', 'sending')] });
  await page.goto('/');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  const queue = page.getByRole('region', { name: '发送队列', exact: true });
  const unknown = queue.getByRole('article', { name: '发送任务：任务 old-unknown · 测试邮箱 1', exact: true });
  await expect(unknown.getByRole('button', { name: '确认未送达并重试', exact: true })).toBeVisible();
  await expect(unknown.getByRole('button', { name: '重试发送', exact: true })).toHaveCount(0);
  await expect(queue.getByRole('article', { name: '发送任务：任务 sending · 测试邮箱 1', exact: true }).getByRole('button')).toHaveCount(0);
  await queue.getByLabel('发送队列账号', { exact: true }).selectOption('real-2');
  await expect(unknown).toHaveCount(0);
  const rejected = queue.getByRole('article', { name: '发送任务：任务 rejected · 测试邮箱 2', exact: true });
  await expect(rejected).toContainText('测试邮箱 2');
  await page.evaluate(() => (window as unknown as { __syncTest: { failNextOutbox: () => void } }).__syncTest.failNextOutbox());
  await rejected.getByRole('button', { name: '重试发送', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('发送状态已变化');
  await expect(rejected.getByRole('button', { name: '重试发送', exact: true })).toBeEnabled();
  await rejected.getByRole('button', { name: '重试发送', exact: true }).click();
  await expect(rejected).toContainText('发送排队中');
  await rejected.getByRole('button', { name: '取消发送', exact: true }).click();
  await expect(rejected).toHaveCount(0);
  await expect(queue).toContainText('正文和附件均保留');
  const actions = await page.evaluate(() => (window as unknown as { __syncTest: { outboxActions: () => Array<{ id: string; action: string }> } }).__syncTest.outboxActions());
  expect(actions).toEqual([{ id: 'rejected', action: 'mail_retry_outbox' }, { id: 'rejected', action: 'mail_outbox_cancel' }]);
});

test('a clearly failed send returns to the same editable draft with recipients, HTML and attachments intact', async ({ page }) => {
  await mockDesktop(page, false, undefined, false, false, true, { outbox: [queueEntry('edit', 'failed', 'real-2')] });
  await page.goto('/');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  const task = page.getByRole('article', { name: '发送任务：任务 edit · 测试邮箱 2', exact: true });
  await task.getByRole('button', { name: '返回草稿修改', exact: true }).click();
  await expect(task).toHaveCount(0);
  await page.getByRole('button', { name: '关闭对话框', exact: true }).click();
  await page.getByRole('button', { name: '编辑草稿', exact: true }).click();
  const compose = page.locator('.compose-pane');
  await expect(compose.getByLabel('发件人', { exact: true })).toHaveValue('real-2');
  await expect(compose.getByLabel('主题', { exact: true })).toHaveValue('任务 edit');
  await expect(compose.getByRole('textbox', { name: '邮件正文' })).toContainText('保留的正文 edit');
  await expect(compose.getByRole('list', { name: '附件列表' })).toContainText('测试附件.txt');
  await compose.getByRole('textbox', { name: '邮件正文' }).fill('退回后修改的正文');
  await expect(compose.getByRole('status').filter({ hasText: '草稿已自动保存' })).toBeVisible();
  const draft = await page.evaluate(() => (window as unknown as { __syncTest: { drafts: () => MailDraft[] } }).__syncTest.drafts().find(draft => draft.id === 'draft-edit'));
  expect(draft).toMatchObject({ id: 'draft-edit', accountId: 'real-2', cc: 'copy@example.test', bcc: 'private@example.test', bodyText: '退回后修改的正文', attachments: [{ name: '测试附件.txt' }] });
});

test('POP3 joins manual sync-all, saves only automatic-sync options, and keeps its protocol when reconfigured', async ({ page }) => {
  await mockDesktop(page, false, undefined, false, false, true, { pop3: true });
  await page.goto('/');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await expect(page.getByRole('button', { name: '同步 测试邮箱 3', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: '同步全部', exact: true }).click();
  await expect(page.getByRole('button', { name: '取消排队 测试邮箱 3', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '取消排队 测试邮箱 3', exact: true }).click();
  await page.getByRole('button', { name: '同步 测试邮箱 3', exact: true }).click();
  await expect(page.getByRole('button', { name: '取消排队 测试邮箱 3', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '同步设置 测试邮箱 3', exact: true }).click();
  const settings = page.getByRole('dialog', { name: '测试邮箱 3 · POP3 同步设置', exact: true });
  await expect(settings.getByRole('checkbox')).toHaveCount(0);
  await expect(settings).toContainText('不删除服务器副本');
  await settings.getByLabel('自动同步', { exact: true }).selectOption('300');
  await settings.getByRole('button', { name: '保存设置', exact: true }).click();
  await page.getByRole('button', { name: '同步设置 测试邮箱 3', exact: true }).click();
  await expect(settings.getByLabel('自动同步', { exact: true })).toHaveValue('300');
  const saved = await page.evaluate(() => (window as unknown as { __syncTest: { settings: () => Record<string, unknown> } }).__syncTest.settings());
  expect(saved).toEqual({ 'real-3': { syncAllFolders: false, historyEnabled: false, autoSyncSeconds: 300 } });
  await settings.getByRole('button', { name: '取消', exact: true }).click();
  await page.getByRole('button', { name: '重新配置 测试邮箱 3', exact: true }).click();
  const wizard = page.getByRole('dialog', { name: '更新 POP3 邮箱连接', exact: true });
  await expect(wizard.getByLabel('POP3 服务器', { exact: true })).toHaveValue('pop.example.test');
  await expect(wizard.getByLabel('POP3 端口', { exact: true })).toHaveValue('995');
  await expect(wizard.getByLabel('POP3 加密', { exact: true }).getByRole('option', { name: /STARTTLS/ })).toHaveCount(0);
  await wizard.getByLabel('密码', { exact: true }).fill('synthetic-only');
  await wizard.getByRole('button', { name: '测试连接', exact: true }).click();
  await expect(wizard).toContainText('POP3连接成功');
  await wizard.getByRole('button', { name: '保存账号', exact: true }).click();
  await expect(wizard).toHaveCount(0);
  const inputs = await page.evaluate(() => (window as unknown as { __syncTest: { accountInputs: () => Array<{ command: string; provider: string }> } }).__syncTest.accountInputs());
  expect(inputs).toEqual([{ command: 'mail_verify_account', provider: 'pop3' }, { command: 'mail_save_account', provider: 'pop3' }]);
});

test('outbox remains readable in each skin and mode at desktop and narrow widths', async ({ page }, info) => {
  test.setTimeout(60000);
  await mockDesktop(page, false, undefined, false, false, true, { pop3: true, outbox: [queueEntry('long-rejection', 'failed'), queueEntry('unknown', 'unknown'), queueEntry('queued', 'queued', 'real-3')] });
  await page.goto('/');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  for (const skin of ['Paper', 'Graphite', 'Linen']) for (const mode of ['浅色', '深色']) {
    await page.getByRole('tab', { name: '外观与皮肤', exact: true }).click();
    await page.getByRole('button', { name: new RegExp(`^使用皮肤 ${skin}`) }).click();
    await page.getByRole('button', { name: mode, exact: true }).click();
    await page.getByRole('tab', { name: '邮箱账号', exact: true }).click();
    for (const width of [1440, 900]) {
      await page.setViewportSize({ width, height: width === 900 ? 700 : 950 });
      const queue = page.getByRole('region', { name: '发送队列', exact: true });
      await queue.scrollIntoViewIfNeeded();
      expect(await queue.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
      await page.screenshot({ path: info.outputPath(`outbox-${skin}-${mode}-${width}.png`) });
    }
  }
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


test('list multi-select batches mark-read and delete into trash', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true);
  await page.goto('/');
  const list = page.getByRole('region', { name: '邮件列表' });
  await expect(list).toBeVisible();
  const rows = list.locator('.mail-row');
  // 选择两封邮件进入批量模式。
  await rows.nth(0).getByRole('button', { name: /选择邮件/ }).click();
  await rows.nth(1).getByRole('button', { name: /选择邮件/ }).click();
  await expect(page.getByText('已选 2 封')).toBeVisible();
  await list.getByRole('button', { name: '标为已读' }).click();
  await expect(page.getByText('已选 2 封')).toHaveCount(0);
  // 批量删除：全部收件箱移除，废纸篓出现两封。
  await rows.nth(0).getByRole('button', { name: /选择邮件/ }).click();
  await rows.nth(1).getByRole('button', { name: /选择邮件/ }).click();
  await list.getByRole('button', { name: '删除', exact: true }).click();
  await expect(page.getByText('已选 2 封')).toHaveCount(0);
  await page.getByRole('navigation', { name: '测试邮箱 1的文件夹' }).getByRole('button', { name: '废纸篓', exact: true }).click();
  await expect(list.getByRole('article')).toHaveCount(2);
  await page.screenshot({ path: info.outputPath('mail-management-trash.png') });
});

test('reader toolbar deletes to trash and moves between folders', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true);
  await page.goto('/');
  const list = page.getByRole('region', { name: '邮件列表' });
  await list.locator('.mail-row').nth(0).getByRole('button', { name: /阅读邮件/ }).click();
  const reader = page.getByRole('region', { name: '邮件阅读区' });
  await expect(reader).toBeVisible();
  await reader.getByRole('button', { name: '移动当前邮件到其他文件夹' }).click();
  await reader.getByRole('menuitem', { name: '移动到 项目邮件' }).click();
  await reader.getByRole('button', { name: '删除当前邮件到废纸篓' }).click();
  await page.getByRole('navigation', { name: '测试邮箱 1的文件夹' }).getByRole('button', { name: '废纸篓', exact: true }).click();
  await expect(list.getByRole('article')).toHaveCount(1);
  await page.screenshot({ path: info.outputPath('mail-management-reader.png') });
});


test('same-subject conversations fold into one row with a count, expand on click and persist the toggle', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true, true, { threads: true });
  await page.goto('/');
  const list = page.getByRole('region', { name: '邮件列表' });
  // 默认关闭：同标题邮件平铺展示。
  await expect(list.getByRole('button', { name: '阅读邮件：季度评审报告', exact: true })).toBeVisible();
  // 开启折叠：Re / 转发前缀归一后 3 封折叠为一行 + 计数徽标；根主题作组标题。
  await list.getByRole('button', { name: '对话', exact: true }).click();
  const group = list.getByRole('button', { name: '展开会话：季度评审报告，共 3 封', exact: true });
  await expect(group).toBeVisible();
  await expect(list.locator('.thread-toggle')).toHaveCount(1);
  await expect(list.getByRole('button', { name: '阅读邮件：季度评审报告', exact: true })).toHaveCount(0);
  // 展开按时间正序，最新一封自动选中；再点折叠（aria-label 随状态切换）。
  await group.click();
  const expandedGroup = list.getByRole('button', { name: '折叠会话：季度评审报告', exact: true });
  await expect(expandedGroup).toBeVisible();
  await expect(expandedGroup).toHaveAttribute('aria-expanded', 'true');
  await expect(list.locator('.thread-child')).toHaveCount(3);
  await expect(list.locator('.thread-child.is-selected')).toContainText('转发：季度评审报告');
  await expandedGroup.click();
  await expect(list.locator('.thread-child')).toHaveCount(0);
  // 开关持久化：刷新后仍为开启状态（展开集合按视图重置为折叠）。
  await page.reload();
  await expect(list.getByRole('button', { name: '展开会话：季度评审报告，共 3 封', exact: true })).toBeVisible();
  // 关闭开关恢复平铺。
  await list.getByRole('button', { name: '对话', exact: true }).click();
  await expect(list.getByRole('button', { name: '阅读邮件：季度评审报告', exact: true })).toBeVisible();
  await expect(list.locator('.thread-toggle')).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('subject-conversations.png') });
});

test('trash view offers permanent delete which queues expunge after confirm', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true);
  await page.goto('/');
  const list = page.getByRole('region', { name: '邮件列表' });
  // 删除两封到废纸篓。
  const rows = list.locator('.mail-row');
  await rows.nth(0).getByRole('button', { name: /选择邮件/ }).click();
  await rows.nth(1).getByRole('button', { name: /选择邮件/ }).click();
  await list.getByRole('button', { name: '删除', exact: true }).click();
  await page.getByRole('navigation', { name: '测试邮箱 1的文件夹' }).getByRole('button', { name: '废纸篓', exact: true }).click();
  await expect(list.getByRole('article')).toHaveCount(2);
  // 批量栏出现「永久删除」；非废纸篓视图不显示（前面收件箱视图的批量栏没有该按钮）。
  await rows.nth(0).getByRole('button', { name: /选择邮件/ }).click();
  await expect(list.getByRole('button', { name: '永久删除', exact: true })).toBeVisible();
  // window.confirm 确认后执行：桩返回 queued，邮件保留在废纸篓（EXPUNGE 未确认前不消失）。
  page.once('dialog', dialog => dialog.accept());
  await list.getByRole('button', { name: '永久删除', exact: true }).click();
  await expect(page.getByText('已选 1 封')).toHaveCount(0);
  const purgeStates = await page.evaluate(() => (window as unknown as { __syncTest: { messages: () => Array<{ changes: Array<{ field: string; state: string }> | null }> } }).__syncTest.messages().flatMap(message => message.changes ?? []).filter(change => change.field === 'purge'));
  expect(purgeStates.length).toBeGreaterThan(0);
  expect(purgeStates.every(change => change.state === 'queued')).toBe(true);
  await page.screenshot({ path: info.outputPath('mail-purge-trash.png') });
});

test('settings folder management creates renames and deletes custom IMAP folders', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true);
  await page.goto('/');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  const accountCard = page.locator('.settings-accounts > div').filter({ hasText: '测试邮箱 1' });
  await accountCard.getByRole('button', { name: '管理 测试邮箱 1 的服务器目录' }).click();
  const dialog = page.getByRole('dialog', { name: '测试邮箱 1 · 服务器目录' });
  await expect(dialog).toBeVisible();
  // 新建目录。
  await dialog.getByRole('textbox', { name: '新建目录名称' }).fill('验收目录');
  await dialog.getByRole('button', { name: '新建', exact: true }).click();
  await expect(dialog.getByText('验收目录')).toBeVisible();
  // 重命名（确认对话框）。
  page.once('dialog', dialog => dialog.accept());
  await dialog.getByRole('button', { name: '重命名目录 验收目录' }).click();
  await dialog.getByRole('textbox', { name: '重命名目录 验收目录' }).fill('验收目录改');
  await dialog.getByRole('button', { name: '重命名', exact: true }).click();
  await expect(dialog.getByText('验收目录改')).toBeVisible();
  // 删除（确认对话框）。
  page.once('dialog', dialog => dialog.accept());
  await dialog.getByRole('button', { name: '删除目录 验收目录改' }).click();
  await expect(dialog.getByText('验收目录改')).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('settings-folder-manager.png') });
});

test('conversation mode links reference chains and the reader navigates within a conversation', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true, true, { threads: true });
  await page.goto('/');
  const list = page.getByRole('region', { name: '邮件列表' });
  await list.getByRole('button', { name: '对话', exact: true }).click();
  // 引用链 + 改主题：与标题归组不同，这里靠 References 连通。
  const chained = list.getByRole('button', { name: '展开会话：立项讨论（原主题），共 2 封', exact: true });
  await expect(chained).toBeVisible();
  await chained.click();
  // 点击组内最新一封（改主题后的跟进），阅读区出现会话导航。
  await list.getByRole('button', { name: '阅读邮件：改主题后的跟进', exact: true }).click();
  const reader = page.getByRole('region', { name: '邮件阅读区' });
  await expect(reader.getByText('2 / 2')).toBeVisible();
  // 上一封：回到根邮件（原主题）。
  await reader.getByRole('button', { name: '会话内上一封' }).click();
  await expect(reader.getByText('1 / 2')).toBeVisible();
  await expect(reader.getByRole('heading', { name: /立项讨论（原主题）/ })).toBeVisible();
  await page.screenshot({ path: info.outputPath('reader-conversation-nav.png') });
});

test('large mailbox list renders only the virtual window and pages in as it scrolls', async ({ page }, info) => {
  await mockDesktop(page, true, undefined, true, true, true, { huge: 600 });
  await page.goto('/');
  const list = page.getByRole('region', { name: '邮件列表' });
  await expect(list).toBeVisible();
  const rendered = await list.locator('.mail-row').count();
  // 首页分页加载（200 封）中只渲染视口窗口（< 120 行）。
  expect(rendered).toBeGreaterThan(0);
  expect(rendered).toBeLessThan(120);
  const scroller = list.locator('.mail-list-scroll');
  const metrics = await scroller.evaluate(element => ({ scrollHeight: element.scrollHeight, clientHeight: element.clientHeight }));
  // 懒加载：初始滚动高度只覆盖已加载的首页，而不是全部 602 封。
  expect(metrics.scrollHeight).toBeGreaterThan(100 * 60);
  expect(metrics.scrollHeight).toBeLessThan(600 * 60);
  // 滚动到底部逐页加载：仍只渲染窗口，最终能看到最早的一封。
  await expect(async () => {
    await scroller.evaluate(element => { element.scrollTop = element.scrollHeight; });
    await expect(list.locator('.list-footer')).toContainText('共 602 封邮件', { timeout: 2000 });
  }).toPass({ timeout: 20000 });
  await expect(list.getByText('大数据量邮件 1', { exact: true })).toBeVisible();
  const after = await list.locator('.mail-row').count();
  expect(after).toBeLessThan(120);
  const fullyLoaded = await scroller.evaluate(element => element.scrollHeight);
  expect(fullyLoaded).toBeGreaterThan(600 * 60);
  // 全部页加载完成后全选覆盖完整数据集。
  await list.getByRole('button', { name: '选择当前列表全部邮件' }).click();
  await expect(page.getByText('已选 602 封', { exact: true })).toBeVisible();
  await page.screenshot({ path: info.outputPath('virtual-large-list.png') });
});
