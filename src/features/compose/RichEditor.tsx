import { useCallback, useEffect, useRef, useState } from 'react';
import { Bold, Italic, Underline, Strikethrough, List, ListOrdered, Quote, Undo2, Redo2, RemoveFormatting, Link2, Minus, AlignLeft, AlignCenter, AlignRight, AlignJustify, Type } from 'lucide-react';

const COLORS = ['#1c2430', '#c0392b', '#d35400', '#b7950b', '#1e8449', '#2471a3', '#7d3c98', '#7f8c8d'];
/** Word 式字号档位（execCommand fontSize 的 1–7 档）。 */
const FONT_SIZES = [
  { value: '2', label: '小' },
  { value: '3', label: '正文' },
  { value: '4', label: '大' },
  { value: '5', label: '特大' },
  { value: '6', label: '超大' },
];
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
  const [empty, setEmpty] = useState(true);

  const emit = useCallback(() => {
    const el = editorRef.current;
    if (!el) return;
    setEmpty(el.innerText.trim() === '');
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
        justifyLeft: document.queryCommandState('justifyLeft'),
        justifyCenter: document.queryCommandState('justifyCenter'),
        justifyRight: document.queryCommandState('justifyRight'),
        justifyFull: document.queryCommandState('justifyFull'),
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
    // 回车产生 <p> 段落（Word 行为），而不是浏览器默认的 <div>。
    document.execCommand('defaultParagraphSeparator', false, 'p');
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

  // 从 Word / 网页粘贴：保留格式，但剥离图片（撰写不支持附件图片，避免保存后凭空消失）。
  function onPaste() {
    const el = editorRef.current;
    if (!el) return;
    window.setTimeout(() => {
      el.querySelectorAll('img').forEach(node => node.remove());
      emit();
    }, 0);
  }

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
        <select aria-label="字号" value="" disabled={disabled} onChange={event => { if (event.target.value) exec('fontSize', event.target.value); }}>
          <option value="">字号</option>
          {FONT_SIZES.map(size => <option key={size.value} value={size.value}>{size.label}</option>)}
        </select>
      </label>
      <label className="toolbar-block">
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
      {toolbarButton('justifyLeft', AlignLeft, '左对齐', 'justifyLeft')}
      {toolbarButton('justifyCenter', AlignCenter, '居中', 'justifyCenter')}
      {toolbarButton('justifyRight', AlignRight, '右对齐', 'justifyRight')}
      {toolbarButton('justifyFull', AlignJustify, '两端对齐', 'justifyFull')}
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
      {toolbarButton('insertHorizontalRule', Minus, '插入分割线')}
      {toolbarButton('removeFormat', RemoveFormatting, '清除格式')}
      <span className="toolbar-divider" />
      {toolbarButton('undo', Undo2, '撤销')}
      {toolbarButton('redo', Redo2, '重做')}
    </div>
    <div className="compose-editor-frame">
      {empty && <div className="compose-placeholder" aria-hidden>在这里写下邮件正文…</div>}
      <div ref={editorRef} className="compose-editor" contentEditable={!disabled} suppressContentEditableWarning role="textbox" aria-multiline aria-label="邮件正文" spellCheck onInput={emit} onBlur={emit} onFocus={syncStates} onPaste={onPaste} />
    </div>
  </div>;
}

export { escapeHtml };
