import test from 'node:test';
import assert from 'node:assert/strict';
import { linkifyText } from '../src/domain/linkify';

const kinds = (text: string) => linkifyText(text).map(segment => segment.kind);
const values = (text: string) => linkifyText(text).map(segment => (segment.kind === 'link' ? segment.href : segment.value));

test('urls keep punctuation and unbalanced parens out of the link', () => {
  assert.deepEqual(values('如有问题请通过 https://example.com/a。联系我们'), [
    '如有问题请通过 ', 'https://example.com/a', '。联系我们',
  ]);
  assert.deepEqual(values('在浏览器中查看：(https://dashboard.example.com/receipt/CAcQARoX)'), [
    '在浏览器中查看：(', 'https://dashboard.example.com/receipt/CAcQARoX', ')',
  ]);
  // 平衡括号内的逗号属于 URL（wiki 风格），完整保留。
  assert.deepEqual(values('see https://en.wikipedia.org/wiki/Rust_(programming_language) ok'), [
    'see ', 'https://en.wikipedia.org/wiki/Rust_(programming_language)', ' ok',
  ]);
});

test('email addresses become mailto links without swallowing surrounding text', () => {
  assert.deepEqual(values('请联系 help@anoma.ly (中文客服) 或致电'), [
    '请联系 ', 'mailto:help@anoma.ly', ' (中文客服) 或致电',
  ]);
  assert.deepEqual(values('邮箱 alex.smith+noreply@example.co.uk,电话不匹配'), [
    '邮箱 ', 'mailto:alex.smith+noreply@example.co.uk', ',电话不匹配',
  ]);
});

test('tel numbers and other schemes stay plain text', () => {
  const text = '致电 +1 415-712-4747 (tel:+14157124747)。';
  assert.equal(linkifyText(text).filter(segment => segment.kind === 'link').length, 0);
  const html = 'ftp://files.example.com 和 file:///etc/passwd 不链接';
  assert.equal(linkifyText(html).filter(segment => segment.kind === 'link').length, 0);
});

test('urls stop at cjk text without a separator', () => {
  const text = '访问 https://example.com/item12查看详情';
  const result = linkifyText(text);
  assert.deepEqual(values(text), ['访问 ', 'https://example.com/item12', '查看详情']);
  assert.equal(result.length, 3);
});

test('kinds alternate text and links with original text preserved', () => {
  const text = 'https://a.example 明文 help@b.example 结尾';
  assert.deepEqual(kinds(text), ['link', 'text', 'link', 'text']);
  const display = linkifyText(text).map(segment => segment.value).join('');
  assert.equal(display, text);
});
