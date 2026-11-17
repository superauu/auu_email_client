import test from 'node:test';
import assert from 'node:assert/strict';
import { virtualWindow } from '../src/features/mail/virtual';

test('virtual window renders only the visible slice with overscan and pads the rest', () => {
  // 600 行、行高 76、视口 760px（约 10 行）+ overscan 8。
  const view = virtualWindow({ count: 600, rowHeight: 76, scrollTop: 0, viewportHeight: 760, overscan: 8 });
  assert.equal(view.start, 0);
  assert.equal(view.end, 19); // 10 可见 + 1 余量 + 8 overscan
  assert.equal(view.topPad, 0);
  assert.equal(view.bottomPad, (600 - 19) * 76);
  // 渲染行数远小于总数。
  assert.ok(view.end - view.start < 30);
});

test('virtual window tracks scroll position and clamps at the end', () => {
  const middle = virtualWindow({ count: 600, rowHeight: 76, scrollTop: 76 * 300, viewportHeight: 760, overscan: 8 });
  assert.equal(middle.start, 292);
  assert.equal(middle.topPad, 292 * 76);
  assert.ok(middle.end <= 600);
  const bottom = virtualWindow({ count: 600, rowHeight: 76, scrollTop: 76 * 590, viewportHeight: 760, overscan: 8 });
  assert.equal(bottom.end, 600);
  assert.equal(bottom.bottomPad, 0);
  assert.ok(bottom.start > 500);
});

test('virtual window handles empty lists and zero measurement', () => {
  assert.deepEqual(virtualWindow({ count: 0, rowHeight: 76, scrollTop: 0, viewportHeight: 760 }), { start: 0, end: 0, topPad: 0, bottomPad: 0 });
  assert.deepEqual(virtualWindow({ count: 10, rowHeight: 0, scrollTop: 0, viewportHeight: 760 }), { start: 0, end: 0, topPad: 0, bottomPad: 0 });
  // 视口尚未测量（0）时至少渲染 overscan 窗口，避免首帧空白。
  const unmeasured = virtualWindow({ count: 10, rowHeight: 76, scrollTop: 0, viewportHeight: 0, overscan: 8 });
  assert.deepEqual(unmeasured, { start: 0, end: 9, topPad: 0, bottomPad: 76 });
});
