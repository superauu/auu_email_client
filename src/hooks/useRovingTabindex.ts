import { useCallback, useRef } from 'react';

/**
 * 为手写工具栏提供 roving tabindex：仅当前焦点项 tabIndex=0，其余 -1；
 * 方向键 / Home / End 在可聚焦子元素间移动焦点。
 */
export function useRovingTabindex(orientation: 'horizontal' | 'vertical' = 'horizontal') {
  const containerRef = useRef<HTMLElement | null>(null);

  const focusables = useCallback((): HTMLElement[] => {
    const root = containerRef.current;
    if (!root) return [];
    return Array.from(root.querySelectorAll<HTMLElement>('button:not([disabled]), [tabindex]:not([tabindex="-1"])'))
      .filter(el => !el.hasAttribute('disabled'));
  }, []);

  const onKeyDown = useCallback((event: React.KeyboardEvent) => {
    // 子元素（Radix 菜单触发键、移动菜单触发键）已消费的方向键不再劫持焦点。
    if (event.defaultPrevented) return;
    const items = focusables();
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLElement);
    const prevKey = orientation === 'horizontal' ? 'ArrowLeft' : 'ArrowUp';
    const nextKey = orientation === 'horizontal' ? 'ArrowRight' : 'ArrowDown';
    let next = current;
    if (event.key === prevKey) next = current <= 0 ? items.length - 1 : current - 1;
    else if (event.key === nextKey) next = current < 0 || current === items.length - 1 ? 0 : current + 1;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = items.length - 1;
    else return;
    event.preventDefault();
    const target = items[next];
    if (target) {
      items.forEach((el, i) => el.setAttribute('tabindex', i === next ? '0' : '-1'));
      target.focus();
    }
  }, [focusables, orientation]);

  // 焦点从容器外进入时初始化 roving 起点（以当前获焦项为锚）；容器内部移动不重置，
  // 避免每次子元素获焦都把 Tab 循环拉回第一项。
  const onFocus = useCallback((event: React.FocusEvent) => {
    const root = containerRef.current;
    if (!root) return;
    if (event.relatedTarget instanceof Node && root.contains(event.relatedTarget)) return;
    const items = focusables();
    if (items.length === 0) return;
    const active = document.activeElement instanceof HTMLElement && items.includes(document.activeElement)
      ? items.indexOf(document.activeElement)
      : 0;
    items.forEach((el, i) => el.setAttribute('tabindex', i === active ? '0' : '-1'));
  }, [focusables]);

  return { containerRef, onKeyDown, onFocus };
}