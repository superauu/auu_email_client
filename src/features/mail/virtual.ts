/**
 * 列表虚拟化窗口计算（纯函数，便于单测）。
 * 只渲染视口内行 + 前后 overscan，用上下留白占位保持滚动条高度。
 */
export interface VirtualWindow {
  start: number;
  end: number;
  topPad: number;
  bottomPad: number;
}

export function virtualWindow(params: {
  count: number;
  rowHeight: number;
  scrollTop: number;
  viewportHeight: number;
  overscan?: number;
}): VirtualWindow {
  const { count, rowHeight, scrollTop, viewportHeight } = params;
  const overscan = Math.max(0, Math.floor(params.overscan ?? 8));
  if (count <= 0 || rowHeight <= 0) return { start: 0, end: 0, topPad: 0, bottomPad: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visible = Math.ceil(Math.max(0, viewportHeight) / rowHeight) + 1;
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, first + visible + overscan);
  return {
    start,
    end,
    topPad: start * rowHeight,
    bottomPad: (count - end) * rowHeight,
  };
}
