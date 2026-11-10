import type { MailMessage, WorkspaceSnapshot } from '../../domain/mail';

const capabilities = { folders: true, serverLabels: true, readStateSync: true, threads: true, calendar: true };
const account = 'personal';
function message(id: string, overrides: Partial<MailMessage>): MailMessage {
  return {
    id, accountId: account, threadId: id, mailbox: 'inbox', from: { name: '林知夏', email: 'zhixia@example.test' },
    to: ['alex@example.test'], subject: '', preview: '', bodyText: '', receivedAt: '2026-11-10T02:00:00Z',
    unread: false, starred: false, labelIds: [], ...overrides,
  };
}

/** RFC-reserved domains only. These accounts never connect to a mail server. */
export const demoWorkspace: WorkspaceSnapshot = {
  accounts: [
    { id: account, name: '个人邮箱', email: 'alex@example.test', avatar: 'A', color: 'teal', provider: 'gmail', status: 'demo', capabilities },
    { id: 'work', name: '工作邮箱', email: 'alex@studio.example', avatar: 'S', color: 'blue', provider: 'microsoft', status: 'demo', capabilities },
    { id: 'other', name: '独立邮箱', email: 'hello@mail.example', avatar: 'H', color: 'amber', provider: 'imap', status: 'demo', capabilities: { ...capabilities, serverLabels: false, calendar: false } },
  ],
  labels: [
    { id: 'design', name: '设计灵感', color: 'teal', scope: 'local' },
    { id: 'project', name: '项目协作', color: 'blue', scope: 'local' },
    { id: 'life', name: '生活日常', color: 'amber', scope: 'local' },
  ],
  messages: [
    message('design-review', {
      accountId: 'work', subject: '让下一版界面，多一点呼吸感', preview: '整理了本周的设计方向，想和你一起看看阅读体验还有哪些可能。',
      bodyText: 'Hi Alex，\n\n整理了本周的设计方向，想和你一起看看阅读体验还有哪些可能。\n\n这次想从「让内容成为主角」出发，减少不必要的装饰，让层次、留白和文字本身构成界面的秩序。\n\n几个可以一起讨论的方向：\n\n1. 更舒展的阅读区域，让长邮件也能轻松读完。\n2. 温和的背景与强调色，让重要信息自然浮现。\n3. 保留足够清晰的账号标识，让切换邮箱变得安心。\n\n周三下午我们可以一起过一遍设计稿。也欢迎你提前把想法回复给我。\n\n期待你的反馈。\n\n知夏\nProduct Designer · Studio',
      unread: true, starred: true, labelIds: ['design', 'project'], receivedAt: '2026-11-10T03:24:00Z',
    }),
    message('weekly', {
      accountId: 'work', from: { name: '陈亦舟', email: 'yizhou@studio.example' }, subject: '本周项目进展与下一步安排',
      preview: '同步一下本周进展：基础组件已经完成，接下来一起打磨细节。', bodyText: 'Alex，\n\n同步一下本周进展：基础组件已经完成。\n\n接下来我们会集中处理阅读体验和账号切换，周五之前整理好反馈。\n\n谢谢！\n亦舟',
      unread: true, labelIds: ['project'], receivedAt: '2026-11-10T01:42:00Z',
    }),
    message('newsletter', {
      from: { name: 'Design Letters', email: 'letters@design.example' }, subject: '好的工具，让你忘记工具的存在',
      preview: '本期的灵感来自日常：一张纸、一盏灯，以及那些安静但有力量的设计。',
      bodyText: '写给热爱设计的你：\n\n好的工具，让你忘记工具的存在。\n\n这一期，我们关注细小但重要的体验：恰当的字号、有节奏的留白，以及顺手的快捷键。\n\n愿你今天也能找到一点新的灵感。\n\nDesign Letters',
      starred: true, labelIds: ['design'], receivedAt: '2026-11-10T00:16:00Z',
    }),
    message('coffee', {
      from: { name: '许小满', email: 'xiaoman@example.test' }, subject: '周末，一起去喝杯咖啡？',
      preview: '发现了一家很棒的小店，窗边的位置刚好可以晒到下午的太阳。',
      bodyText: 'Alex！\n\n发现了一家很棒的小店，窗边的位置刚好可以晒到下午的太阳。\n\n这周六下午有空吗？一起喝杯咖啡，聊聊最近的生活吧。\n\n小满',
      unread: true, labelIds: ['life'], receivedAt: '2026-11-09T23:30:00Z',
    }),
    message('domain', {
      accountId: 'other', from: { name: 'Mail Service', email: 'notice@mail.example' }, subject: '你的独立邮箱已经准备就绪',
      preview: '欢迎使用独立邮箱。从这里开始，让每一次交流都有自己的风格。',
      bodyText: '你好，\n\n你的独立邮箱已经准备就绪。\n\n你可以使用自己的域名收发邮件，整理文件夹并设置签名。\n\nMail Service',
      receivedAt: '2026-11-09T08:20:00Z',
    }),
    message('reading', {
      from: { name: '慢读俱乐部', email: 'reading@books.example' }, subject: '十一月书单：把时间留给喜欢的事',
      preview: '三本适合在秋日翻开的书，还有一份慢下来的邀请。',
      bodyText: '亲爱的读者，\n\n这个月，我们挑选了三本关于生活、创造与专注的书。\n\n给自己留一点时间，关掉通知，读几页喜欢的文字。\n\n慢读俱乐部',
      labelIds: ['life'], receivedAt: '2026-11-09T06:05:00Z',
    }),
    message('sent-note', { mailbox: 'sent', from: { name: 'Alex', email: 'alex@example.test' }, to: ['xiaoman@example.test'], subject: 'Re: 上次的照片', preview: '照片收到了，很喜欢那天的光线。', bodyText: '照片收到了，很喜欢那天的光线。谢谢分享！', receivedAt: '2026-11-08T05:00:00Z' }),
    message('archived-note', { mailbox: 'archive', subject: '十月设计回顾', preview: '已整理的十月设计记录。', bodyText: '十月设计记录已经归档，随时回来看看。', labelIds: ['design'], receivedAt: '2026-11-01T01:00:00Z' }),
  ],
  events: [
    { id: 'review', accountId: 'work', messageId: 'design-review', title: '阅读体验 · 设计讨论', startsAt: '2026-11-11T04:00:00Z', timeZone: 'Australia/Sydney', completed: false, source: 'local' },
    { id: 'feedback', accountId: 'work', messageId: 'weekly', title: '整理本周项目反馈', startsAt: '2026-11-13T03:00:00Z', timeZone: 'Australia/Sydney', completed: false, source: 'local' },
  ],
};
