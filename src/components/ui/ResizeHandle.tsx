import { useRef } from 'react';
import { useLayoutStore, MIN_LIST_WIDTH, MAX_LIST_WIDTH } from '../../stores/layoutStore';

export function ResizeHandle() {
  const width = useLayoutStore(s => s.listWidth);
  const setWidth = useLayoutStore(s => s.setListWidth);
  const start = useRef<{ x: number; width: number } | null>(null);
  return <div role="separator" aria-label="调整邮件列表宽度" aria-orientation="vertical" aria-valuemin={MIN_LIST_WIDTH} aria-valuemax={MAX_LIST_WIDTH} aria-valuenow={width} tabIndex={0} className="resize-handle"
    onKeyDown={event => {
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setWidth(width + (event.key === 'ArrowRight' ? 20 : -20)); }
      if (event.key === 'Home') { event.preventDefault(); setWidth(MIN_LIST_WIDTH); }
      if (event.key === 'End') { event.preventDefault(); setWidth(MAX_LIST_WIDTH); }
    }}
    onPointerDown={event => { if (event.button === 0) { start.current = { x: event.clientX, width }; event.currentTarget.setPointerCapture(event.pointerId); } }}
    onPointerMove={event => { if (start.current) setWidth(start.current.width + event.clientX - start.current.x); }}
    onPointerUp={() => { start.current = null; }} onPointerCancel={() => { start.current = null; }} onLostPointerCapture={() => { start.current = null; }} />;
}
