import { useCallback, useEffect, useRef, useState } from 'react';
import { Bold, Italic, Underline, Strikethrough, List, ListOrdered, Quote, Undo2, Redo2, RemoveFormatting, Link2, AlignLeft, AlignCenter, AlignRight, AlignJustify, Minus, Type, X } from 'lucide-react';

const SAFE_LINK = /^(https?:\/\/|mailto:)[^\s<>"']+$/i;
const COMMANDS = ['bold', 'italic', 'underline', 'strikeThrough', 'insertUnorderedList', 'insertOrderedList', 'justifyLeft', 'justifyCenter', 'justifyRight', 'justifyFull'];

export function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** Filter before insertion so pasted HTML cannot introduce app styles or remote resources. */
function cleanHtml(html: string): string {
  const template = document.createElement('template');
  template.innerHTML = html;
  template.content.querySelectorAll('script,style,iframe,object,embed,svg,math,form,input,button,meta,link,img,video,audio,source').forEach(node => node.remove());
  const tags = new Set('p div span br hr b strong i em u s strike a h1 h2 h3 ul ol li blockquote pre code table thead tbody tfoot caption colgroup col tr td th font center small sub sup'.split(' '));
  for (const element of [...template.content.querySelectorAll('*')]) {
    if (!tags.has(element.tagName.toLowerCase())) { element.replaceWith(...element.childNodes); continue; }
    if (element instanceof HTMLElement) {
      // Email layout attributes are lower priority than the app's reset CSS. Materialize
      // them as local styles before removing attributes, so presentation tables stay intact.
      const length = (value: string | null) => value && /^\d+(?:\.\d+)?%?$/.test(value) ? /%$/.test(value) ? value : `${value}px` : '';
      const set = (property: string, value: string | null) => { if (value && !element.style.getPropertyValue(property)) element.style.setProperty(property, value); };
      set('width', length(element.getAttribute('width'))); set('height', length(element.getAttribute('height')));
      const background = element.getAttribute('bgcolor');
      if (background && /^[#a-z\d]+$/i.test(background)) set('background-color', background);
      const align = element.getAttribute('align');
      if (element.tagName === 'TABLE' && align === 'center') { set('margin-left', 'auto'); set('margin-right', 'auto'); }
      else if (align && /^(left|center|right|justify)$/.test(align)) set('text-align', align);
      const valign = element.getAttribute('valign');
      if (valign && /^(top|middle|bottom|baseline)$/.test(valign)) set('vertical-align', valign);
      if (element.tagName === 'CENTER') set('text-align', 'center');
      if (element.tagName === 'TABLE') {
        const spacing = length(element.getAttribute('cellspacing'));
        if (spacing) { set('border-spacing', spacing); set('border-collapse', 'separate'); }
      }
      if (['TD', 'TH'].includes(element.tagName)) {
        const table = element.closest('table');
        const padding = length(table?.getAttribute('cellpadding') ?? null);
        if (padding) for (const side of ['top', 'right', 'bottom', 'left']) set(`padding-${side}`, padding);
      }
      const tableBorder = element.tagName === 'TABLE' ? element.getAttribute('border') : ['TD', 'TH'].includes(element.tagName) ? element.closest('table')?.getAttribute('border') : null;
      if (tableBorder && /^\d{1,2}$/.test(tableBorder)) for (const side of ['top', 'right', 'bottom', 'left']) { set(`border-${side}-width`, `${tableBorder}px`); set(`border-${side}-style`, 'solid'); }
    }
    for (const attribute of [...element.attributes]) {
      const allowed = (element.tagName === 'A' && attribute.name === 'href' && SAFE_LINK.test(attribute.value)) ||
        (element.tagName === 'FONT' && ['color', 'face', 'size'].includes(attribute.name) && /^[\w\s#.,'-]+$/.test(attribute.value)) ||
        (element.tagName === 'BLOCKQUOTE' && attribute.name === 'data-auumail-quote' && attribute.value === 'true') ||
        (['TABLE', 'TD', 'TH', 'TR', 'COL'].includes(element.tagName) && ['width', 'height', 'cellpadding', 'cellspacing', 'border', 'bgcolor', 'align', 'valign'].includes(attribute.name) && /^[#a-z\d%.]+$/i.test(attribute.value)) ||
        (['TD', 'TH'].includes(element.tagName) && ['colspan', 'rowspan'].includes(attribute.name) && /^\d{1,2}$/.test(attribute.value));
      if (attribute.name !== 'style' && !allowed) element.removeAttribute(attribute.name);
    }
    if (element instanceof HTMLElement) {
      const properties = ['color', 'background-color', 'font-family', 'font-size', 'font-weight', 'font-style', 'text-decoration', 'text-align', 'text-indent', 'text-transform', 'line-height', 'letter-spacing', 'word-spacing', 'white-space', 'vertical-align', 'display', 'width', 'min-width', 'max-width', 'height', 'min-height', 'max-height', 'border-collapse', 'border-spacing', 'table-layout', 'list-style-type', 'list-style-position', 'border-radius'];
      for (const side of ['top', 'right', 'bottom', 'left']) properties.push(`padding-${side}`, `margin-${side}`, `border-${side}-width`, `border-${side}-style`, `border-${side}-color`);
      const safe = properties.map(property => {
        const value = element.style.getPropertyValue(property);
        return value && !/url|var\(|expression|[<>@\\]/i.test(value) ? `${property}:${value}` : '';
      }).filter(Boolean).join(';');
      element.removeAttribute('style');
      if (safe) element.setAttribute('style', safe);
    }
  }
  return template.innerHTML;
}

export function RichEditor({ initialHtml, initialText, quoteHtml, onChange, disabled, formatting = true }: {
  initialHtml: string; initialText: string; quoteHtml?: string | null;
  onChange: (html: string, text: string) => void; disabled?: boolean; formatting?: boolean;
}) {
  const editorRef = useRef<HTMLDivElement>(null);
  const selection = useRef<Range | null>(null);
  const [active, setActive] = useState<Record<string, boolean>>({});
  const [block, setBlock] = useState('p');
  const [fontSize, setFontSize] = useState('3');
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkUrl, setLinkUrl] = useState('');
  const [linkText, setLinkText] = useState('');
  const [empty, setEmpty] = useState(!initialHtml && !initialText);
  const [pasteNotice, setPasteNotice] = useState('');
  const changeRef = useRef(onChange);
  useEffect(() => { changeRef.current = onChange; }, [onChange]);

  const emit = useCallback(() => {
    const el = editorRef.current;
    if (!el) return;
    const text = el.innerText.replace(/\n+$/, '');
    setEmpty(text.trim() === '' && !el.querySelector('hr,table'));
    changeRef.current(text.trim() || el.querySelector('hr,table') ? el.innerHTML : '', text);
  }, []);
  const syncStates = useCallback(() => {
    const el = editorRef.current;
    const current = window.getSelection();
    if (!el || !current?.rangeCount || !el.contains(current.anchorNode) || !el.contains(current.focusNode)) return;
    selection.current = current.getRangeAt(0).cloneRange();
    setActive(Object.fromEntries(COMMANDS.map(command => [command, document.queryCommandState(command)])));
    setBlock((document.queryCommandValue('formatBlock') || 'p').toLowerCase().replace(/[<>]/g, ''));
    setFontSize(document.queryCommandValue('fontSize') || '3');
  }, []);
  const restoreSelection = useCallback(() => {
    const el = editorRef.current;
    const saved = selection.current;
    el?.focus();
    if (saved && el?.contains(saved.commonAncestorContainer)) {
      const current = window.getSelection();
      current?.removeAllRanges(); current?.addRange(saved);
      selection.current = saved;
    }
  }, []);
  const exec = useCallback((command: string, value?: string) => {
    if (disabled) return;
    restoreSelection(); document.execCommand(command, false, value); emit(); syncStates();
  }, [disabled, restoreSelection, emit, syncStates]);

  useEffect(() => {
    const el = editorRef.current;
    if (!el) return;
    document.execCommand('defaultParagraphSeparator', false, 'p');
    // Opening the editor is not an edit and must not overwrite a plain text draft.
    el.innerHTML = initialHtml ? cleanHtml(initialHtml) : initialText ? `<p>${escapeHtml(initialText).replace(/\n/g, '<br>')}</p>` : '<p><br></p>';
    // eslint-disable-next-line react-hooks/exhaustive-deps -- Parent keys editor by draft identity.
  }, []);
  const injected = useRef(false);
  useEffect(() => {
    if (injected.current || !quoteHtml) return;
    injected.current = true;
    editorRef.current?.insertAdjacentHTML('beforeend', cleanHtml(quoteHtml));
    emit();
  }, [quoteHtml, emit]);
  useEffect(() => {
    document.addEventListener('selectionchange', syncStates);
    return () => document.removeEventListener('selectionchange', syncStates);
  }, [syncStates]);

  function insertLink() {
    const url = linkUrl.trim();
    if (!SAFE_LINK.test(url)) return;
    restoreSelection();
    if (selection.current && !selection.current.collapsed) exec('createLink', url);
    else exec('insertHTML', `<a href="${escapeHtml(url)}">${escapeHtml(linkText.trim() || url)}</a>`);
    setLinkOpen(false); setLinkUrl(''); setLinkText('');
  }
  const tool = (command: string, Icon: typeof Bold, label: string, toggle = false) => <button type="button" title={label} aria-label={label} aria-pressed={toggle ? active[command] ?? false : undefined} className={active[command] ? 'is-active' : ''} disabled={disabled} onMouseDown={event => event.preventDefault()} onClick={() => exec(command)}><Icon size={16} /></button>;

  return <div className="compose-rich">
    <div className="compose-format-panel" hidden={!formatting}>
      <div className="compose-toolbar" role="toolbar" aria-label="格式工具栏">
        <select aria-label="段落样式" value={block} disabled={disabled} onChange={event => exec('formatBlock', event.target.value)}><option value="p">正文</option><option value="h1">标题 1</option><option value="h2">标题 2</option><option value="h3">标题 3</option><option value="blockquote">引用</option></select>
        <select aria-label="字号" value={fontSize} disabled={disabled} onChange={event => exec('fontSize', event.target.value)}><option value="2">10</option><option value="3">12</option><option value="4">14</option><option value="5">18</option><option value="6">24</option></select>
        <span className="toolbar-divider" />
        {tool('bold', Bold, '加粗 (Ctrl+B)', true)}{tool('italic', Italic, '斜体 (Ctrl+I)', true)}{tool('underline', Underline, '下划线 (Ctrl+U)', true)}{tool('strikeThrough', Strikethrough, '删除线', true)}
        <label className="toolbar-color" title="文字颜色"><Type size={16} /><input type="color" aria-label="文字颜色" disabled={disabled} onFocus={event => { const color = document.queryCommandValue('foreColor'); if (/^#[a-f\d]{6}$/i.test(color)) event.target.value = color; }} onChange={event => exec('foreColor', event.target.value)} /></label>
        <span className="toolbar-divider" />
        {tool('insertUnorderedList', List, '项目符号列表', true)}{tool('insertOrderedList', ListOrdered, '编号列表', true)}
        {tool('justifyLeft', AlignLeft, '左对齐', true)}{tool('justifyCenter', AlignCenter, '居中', true)}{tool('justifyRight', AlignRight, '右对齐', true)}{tool('justifyFull', AlignJustify, '两端对齐', true)}
        <button type="button" title="引用" aria-label="引用" disabled={disabled} onMouseDown={event => event.preventDefault()} onClick={() => exec('formatBlock', 'blockquote')}><Quote size={16} /></button>
        <button type="button" title="插入链接" aria-label="插入链接" aria-expanded={linkOpen} disabled={disabled} onMouseDown={event => event.preventDefault()} onClick={() => { setLinkText(window.getSelection()?.toString() ?? ''); setLinkOpen(value => !value); }}><Link2 size={16} /></button>
        {tool('insertHorizontalRule', Minus, '插入分割线')}{tool('removeFormat', RemoveFormatting, '清除格式')}
        <span className="toolbar-divider" />{tool('undo', Undo2, '撤销')}{tool('redo', Redo2, '重做')}
      </div>
      {linkOpen && <div className="toolbar-link" role="group" aria-label="插入链接" onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); setLinkOpen(false); restoreSelection(); } else if (event.key === 'Enter') { event.preventDefault(); insertLink(); } }}>
        <label>显示文字<input aria-label="链接显示文字" value={linkText} onChange={event => setLinkText(event.target.value)} /></label>
        <label>链接地址<input aria-label="链接地址" value={linkUrl} placeholder="https://example.com" onChange={event => setLinkUrl(event.target.value)} autoFocus /></label>
        <div><button type="button" onClick={() => { setLinkOpen(false); restoreSelection(); }}>取消</button><button type="button" onClick={insertLink} disabled={!SAFE_LINK.test(linkUrl.trim())}>插入</button></div>
      </div>}
    </div>
    <div className="compose-editor-frame">
      {empty && <div className="compose-placeholder" aria-hidden>在这里写下邮件正文…</div>}
      <div ref={editorRef} className="compose-editor" contentEditable={!disabled} suppressContentEditableWarning role="textbox" aria-multiline aria-label="邮件正文" spellCheck onInput={emit} onFocus={syncStates}
        onPaste={event => {
          event.preventDefault();
          const html = event.clipboardData.getData('text/html');
          const text = event.clipboardData.getData('text/plain');
          if ([...event.clipboardData.items].some(item => item.kind === 'file') || /<img\b/i.test(html)) setPasteNotice('图片未插入：目前仅支持文字与格式粘贴。');
          if (html) exec('insertHTML', cleanHtml(html));
          else if (text) exec('insertHTML', escapeHtml(text).replace(/\r?\n/g, '<br>'));
        }} />
    </div>
    {pasteNotice && <div className="compose-editor-notice" role="status">{pasteNotice}<button type="button" aria-label="关闭粘贴提示" onClick={() => setPasteNotice('')}><X size={14} /></button></div>}

  </div>;
}
