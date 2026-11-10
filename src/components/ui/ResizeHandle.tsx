import { useRef } from 'react';

/** Vertical workspace splitter: pointer drag, arrow keys, Home / End, double-click resets to default. */
export function ResizeHandle({ label, value, min, max, onResize, onReset, variant }: {
  label: string;
  value: number;
  min: number;
  max: number;
  onResize: (value: number) => void;
  onReset: () => void;
  variant?: 'sidebar';
}) {
  const start = useRef<{ origin: number; width: number } | null>(null);
  return <div role="separator" aria-label={label} aria-orientation="vertical" aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} tabIndex={0}
    className={`resize-handle${variant === 'sidebar' ? ' resize-handle--sidebar' : ''}`}
    onKeyDown={event => {
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); onResize(value + (event.key === 'ArrowRight' ? 20 : -20)); }
      if (event.key === 'Home') { event.preventDefault(); onResize(min); }
      if (event.key === 'End') { event.preventDefault(); onResize(max); }
    }}
    onPointerDown={event => {
      if (event.button !== 0) return;
      // Desktop behavior: a splitter drag never starts a text selection or moves focus ring; keyboard use stays on Tab.
      event.preventDefault();
      event.currentTarget.focus();
      start.current = { origin: event.clientX, width: value };
      event.currentTarget.setPointerCapture(event.pointerId);
      document.body.classList.add('is-resizing');
    }}
    onPointerMove={event => { if (start.current) onResize(start.current.width + event.clientX - start.current.origin); }}
    onPointerUp={event => {
      start.current = null;
      document.body.classList.remove('is-resizing');
      event.currentTarget.releasePointerCapture(event.pointerId);
    }}
    onPointerCancel={() => { start.current = null; document.body.classList.remove('is-resizing'); }}
    onLostPointerCapture={() => { start.current = null; document.body.classList.remove('is-resizing'); }}
    onDoubleClick={event => { event.preventDefault(); onReset(); }} />;
}
