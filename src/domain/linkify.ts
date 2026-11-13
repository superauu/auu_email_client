// 纯文本正文链接化：URL / 邮箱地址切分为可点击段落。
// 设计取舍：URL 仅匹配 ASCII 字符集（正文里的真实链接几乎都是百分号编码），
// 遇到中文立即截断；尾随标点与不成对括号归还给文本。
export type LinkifySegment =
  | { kind: 'text'; value: string }
  | { kind: 'link'; value: string; href: string };

const URL_PATTERN = /https?:\/\/[A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=%]+/gi;
const EMAIL_PATTERN = /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g;
const URL_TRAILERS = new Set(['.', ',', ';', ':', '!', '?', '"', "'", ')', ']', '}', '>', '…', '。', '，', '；', '！', '？', '、', '）']);
const EMAIL_TRAILERS = new Set(['.', ',', '。', '，', '；']);

function trimTrailing(raw: string, trailers: Set<string>): { value: string; trailing: string } {
  let end = raw.length;
  let trailing = '';
  while (end > 0 && trailers.has(raw[end - 1])) {
    // 右括号属于 URL 本身的情况（如 wiki 词条）：仅在不平衡时才剥离。
    if (raw[end - 1] === ')') {
      const head = raw.slice(0, end - 1);
      const opens = (head.match(/\(/g) ?? []).length;
      const closes = (head.match(/\)/g) ?? []).length;
      if (opens > closes) break;
    }
    trailing = raw[end - 1] + trailing;
    end -= 1;
  }
  return { value: raw.slice(0, end), trailing };
}

function isControlFree(value: string): boolean {
  return ![...value].some(ch => {
    const code = ch.charCodeAt(0);
    return code < 0x20 || code === 0x7f;
  });
}

function splitEmails(text: string, out: LinkifySegment[]): void {
  let cursor = 0;
  for (const match of text.matchAll(EMAIL_PATTERN)) {
    const index = match.index ?? 0;
    if (index > cursor) out.push({ kind: 'text', value: text.slice(cursor, index) });
    const { value, trailing } = trimTrailing(match[0], EMAIL_TRAILERS);
    if (value) out.push({ kind: 'link', value, href: `mailto:${value}` });
    if (trailing) out.push({ kind: 'text', value: trailing });
    cursor = index + match[0].length;
  }
  if (cursor < text.length) out.push({ kind: 'text', value: text.slice(cursor) });
}

export function linkifyText(text: string): LinkifySegment[] {
  const segments: LinkifySegment[] = [];
  let cursor = 0;
  for (const match of text.matchAll(URL_PATTERN)) {
    const index = match.index ?? 0;
    if (index > cursor) splitEmails(text.slice(cursor, index), segments);
    const { value, trailing } = trimTrailing(match[0], URL_TRAILERS);
    if (value && isControlFree(value)) segments.push({ kind: 'link', value, href: value });
    else if (value) segments.push({ kind: 'text', value });
    if (trailing) segments.push({ kind: 'text', value: trailing });
    cursor = index + match[0].length;
  }
  if (cursor < text.length) splitEmails(text.slice(cursor), segments);
  return segments;
}
