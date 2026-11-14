import { useCallback, useEffect, useRef, useState } from 'react';
import { Bold, Italic, Underline, Strikethrough, List, ListOrdered, Quote, Undo2, Redo2, RemoveFormatting, Link2, Type } from 'lucide-react';

const COLORS = ['#1c2430', '#c0392b', '#d35400', '#b7950b', '#1e8449', '#2471a3', '#7d3c98', '#7f8c8d'];
const SAFE_LINK = /^(https?:\/\/|mailto:)[^\s]+$/i;

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

export function RichEditor({ initialHtml, quoteHtml, onChange, disabled }: {
  initialHtml: string;
  /** 异步到达的引用 HTML；到齐后追加一次。 */
  quoteHtml?: string | null;
  onChange: (html: string, text: string) => void;
  disabled?: boolean;
}) {
  const editorRef = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<Record<string, boolean>>({});
  const [block, setBlock] = useState('p');
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkUrl, setLinkUrl] = useState('');
  const [colorOpen, setColorOpen] = useState(false);

  const emit = useCallback(() => {
    const el = editorRef.current;
    if (!el) return;
    onChange(el.innerHTML, el.innerText.replace(/\n+$/, ''));
  }, [onChange]);

  const syncStates = useCallback(() => {
    try {
      setActive({
        bold: document.queryCommandState('bold'),
        italic: document.queryCommandState('italic'),
        underline: document.queryCommandState('underline'),
        strikeThrough: document.queryCommandState('strikeThrough'),
        insertUnorderedList: document.queryCommandState('insertUnorderedList'),
        insertOrderedList: document.queryCommandState('insertOrderedList'),
      });
      const current = (document.queryCommandValue('formatBlock') || 'p').toLowerCase().replace(/[<>]/g, '');
      setBlock(current);
    } catch { /* 某些查询在无选区时抛错，忽略 */ }
  }, []);

  const exec = useCallback((command: string, value?: string) => {
    editorRef.current?.focus();
    document.execCommand(command, false, value);
    emit();
    syncStates();
  }, [emit, syncStates]);

  // 挂载：初始内容 + 同步到达的引用。key 由父组件保证草稿切换时重建。
  useEffect(() => {
    const el = editorRef.current;
    if (!el) return;
    el.innerHTML = initialHtml && initialHtml.trim() ? initialHtml : '<p><br></p>';
    if (quoteHtml) el.insertAdjacentHTML('beforeend', quoteHtml);
    emit();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 仅挂载时初始化
  }, []);

  // 引用异步到齐后追加（仅追加一次）。
  const injected = useRef(false);
  useEffect(() => {
    if (injected.current || !quoteHtml) return;
    injected.current = true;
    editorRef.current?.insertAdjacentHTML('beforeend', quoteHtml);
    emit();
  }, [quoteHtml, emit]);

  useEffect(() => {
    const handler = () => {
      const el = editorRef.current;
      if (el && document.activeElement && el.contains(document.activeElement)) syncStates();
    };
    document.addEventListener('selectionchange', handler);
    return () => document.removeEventListener('selectionchange', handler);
  }, [syncStates]);

  function insertLink() {
    const url = linkUrl.trim();
    if (!SAFE_LINK.test(url)) return;
    exec('createLink', url);
    setLinkOpen(false);
    setLinkUrl('');
  }

  const toolbarButton = (command: string, Icon: typeof Bold, label: string, key?: string) => (
    <button type="button" className={key && active[key] ? 'is-active' : ''} aria-label={label} aria-pressed={key ? active[key] ?? false : undefined} disabled={disabled} onMouseDown={event => { event.preventDefault(); exec(command); }}><Icon size={15} /></button>
  );

  return <div className="compose-rich">
    <div className="compose-toolbar" role="toolbar" aria-label="格式工具栏">
      {toolbarButton('bold', Bold, '加粗', 'bold')}
      {toolbarButton('italic', Italic, '斜体', 'italic')}
      {toolbarButton('underline', Underline, '下划线', 'underline')}
      {toolbarButton('strikeThrough', Strikethrough, '删除线', 'strikeThrough')}
      <span className="toolbar-divider" />
      <label className="toolbar-block">
        <Type size={14} aria-hidden />
        <select aria-label="段落样式" value={block} disabled={disabled} onChange={event => exec('formatBlock', `<${event.target.value}>`)}>
          <option value="p">正文</option>
          <option value="h1">标题 1</option>
          <option value="h2">标题 2</option>
          <option value="h3">标题 3</option>
        </select>
      </label>
      {toolbarButton('insertUnorderedList', List, '项目符号列表', 'insertUnorderedList')}
      {toolbarButton('insertOrderedList', ListOrdered, '编号列表', 'insertOrderedList')}
      <button type="button" className={block === 'blockquote' ? 'is-active' : ''} aria-label="引用" aria-pressed={block === 'blockquote'} disabled={disabled} onMouseDown={event => { event.preventDefault(); exec('formatBlock', 'blockquote'); }}><Quote size={15} /></button>
      <span className="toolbar-divider" />
      <div className="toolbar-pop">
        <button type="button" aria-label="文字颜色" aria-expanded={colorOpen} disabled={disabled} onMouseDown={event => { event.preventDefault(); setColorOpen(value => !value); }}><Type size={15} style={{ borderBottom: '3px solid var(--accent)' }} /></button>
        {colorOpen && <div className="toolbar-palette" role="menu" aria-label="选择文字颜色">{COLORS.map(color => <button key={color} type="button" role="menuitem" aria-label={`文字颜色 ${color}`} style={{ background: color }} onMouseDown={event => { event.preventDefault(); exec('foreColor', color); setColorOpen(false); }} />)}</div>}
      </div>
      <div className="toolbar-pop">
        <button type="button" aria-label="插入链接" aria-expanded={linkOpen} disabled={disabled} onMouseDown={event => { event.preventDefault(); setLinkOpen(value => !value); }}><Link2 size={15} /></button>
        {linkOpen && <div className="toolbar-link" role="group" aria-label="插入链接">
          <input aria-label="链接地址" value={linkUrl} placeholder="https://example.com" onChange={event => setLinkUrl(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); insertLink(); } }} autoFocus />
          <button type="button" onClick={insertLink} disabled={!SAFE_LINK.test(linkUrl.trim())}>插入</button>
        </div>}
      </div>
      {toolbarButton('removeFormat', RemoveFormatting, '清除格式')}
      <span className="toolbar-divider" />
      {toolbarButton('undo', Undo2, '撤销')}
      {toolbarButton('redo', Redo2, '重做')}
    </div>
    <div ref={editorRef} className="compose-editor" contentEditable={!disabled} suppressContentEditableWarning role="textbox" aria-multiline aria-label="邮件正文" onInput={emit} onBlur={emit} onFocus={syncStates} />
  </div>;
}

export { escapeHtml };
