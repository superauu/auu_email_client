import { useCallback, useEffect, useRef, useState } from 'react';
import { Bold, Italic, Underline, Strikethrough, List, ListOrdered, Quote, Undo2, Redo2, RemoveFormatting, Link2, AlignLeft, AlignCenter, AlignRight, AlignJustify, Minus, Type, X, Image as ImageIcon } from 'lucide-react';
import { MAX_INLINE_IMAGE } from '../../domain/signature';
import { useI18n } from '../../i18n/react';

const SAFE_LINK = /^(https?:\/\/|mailto:)[^\s<>"']+$/i;
const COMMANDS = ['bold', 'italic', 'underline', 'strikeThrough', 'insertUnorderedList', 'insertOrderedList', 'justifyLeft', 'justifyCenter', 'justifyRight', 'justifyFull'];

export function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** img 仅保留位图 data:image 内嵌（签名 / 正文插图通道）；远程、相对与 svg src 仍剥除。 */
const INLINE_IMAGE_SRC = /^data:image\/(png|jpeg|gif|webp|bmp);base64,/i;

/** Filter before insertion so pasted HTML cannot introduce app styles or remote resources. */
function cleanHtml(html: string): string {
  const template = document.createElement('template');
  template.innerHTML = html;
  template.content.querySelectorAll('script,style,iframe,object,embed,svg,math,form,input,button,meta,link,video,audio,source').forEach(node => node.remove());
  template.content.querySelectorAll('img').forEach(node => {
    if (!INLINE_IMAGE_SRC.test(node.getAttribute('src') ?? '')) node.remove();
  });
  const tags = new Set('p div span br hr b strong i em u s strike a img h1 h2 h3 ul ol li blockquote pre code table thead tbody tfoot caption colgroup col tr td th font center small sub sup'.split(' '));
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
        (element.tagName === 'IMG' && attribute.name === 'src' && INLINE_IMAGE_SRC.test(attribute.value)) ||
        (element.tagName === 'IMG' && attribute.name === 'alt' && /^[^"'<>]*$/.test(attribute.value)) ||
        (element.tagName === 'IMG' && ['width', 'height'].includes(attribute.name) && /^\d+%?$/.test(attribute.value)) ||
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

export function RichEditor({ initialHtml, initialText, quoteHtml, signatureHtml, onPickImage, onChange, disabled, formatting = true }: {
  initialHtml: string; initialText: string; quoteHtml?: string | null;
  /** 账号签名（Rust 消毒后的 HTML，含 data:image 内嵌图）：一次性插入正文末尾。 */
  signatureHtml?: string | null;
  /** 可选「插入图片」按钮（签名编辑器）：Rust 对话框选图后返回 data URI。 */
  onPickImage?: (() => Promise<{ dataUri: string; name?: string } | null>) | null;
  onChange: (html: string, text: string) => void; disabled?: boolean; formatting?: boolean;
}) {
  const t = useI18n();
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
  const [picking, setPicking] = useState(false);
  const changeRef = useRef(onChange);
  useEffect(() => { changeRef.current = onChange; }, [onChange]);

  const emit = useCallback(() => {
    const el = editorRef.current;
    if (!el) return;
    const text = el.innerText.replace(/\n+$/, '');
    // 仅图片（无文字）的正文也算非空，否则占位符叠加且 data URI 在保存时被丢弃。
    setEmpty(text.trim() === '' && !el.querySelector('hr,table,img'));
    changeRef.current(text.trim() || el.querySelector('hr,table,img') ? el.innerHTML : '', text);
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
  const signatureInjected = useRef(false);
  useEffect(() => {
    if (signatureInjected.current || !signatureHtml) return;
    signatureInjected.current = true;
    const el = editorRef.current;
    if (!el) return;
    // 签名是 Rust 消毒后的自家内容，不过 cleanHtml（其 img 剥除针对不可信粘贴）。
    el.insertAdjacentHTML('beforeend', signatureHtml);
    // Outlook 语义：光标置于签名之前，打字内容在签名上方。
    const range = document.createRange();
    range.setStart(el, 0);
    range.collapse(true);
    const current = window.getSelection();
    current?.removeAllRanges(); current?.addRange(range);
    emit();
  }, [signatureHtml, emit]);
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
      <div className="compose-toolbar" role="toolbar" aria-label={t('editor.toolbarAria')}>
        <select aria-label={t('editor.paragraphAria')} value={block} disabled={disabled} onChange={event => exec('formatBlock', event.target.value)}><option value="p">{t('editor.blockP')}</option><option value="h1">{t('editor.blockH1')}</option><option value="h2">{t('editor.blockH2')}</option><option value="h3">{t('editor.blockH3')}</option><option value="blockquote">{t('editor.blockQuote')}</option></select>
        <select aria-label={t('editor.fontSizeAria')} value={fontSize} disabled={disabled} onChange={event => exec('fontSize', event.target.value)}><option value="2">10</option><option value="3">12</option><option value="4">14</option><option value="5">18</option><option value="6">24</option></select>
        <span className="toolbar-divider" />
        {tool('bold', Bold, t('editor.bold'), true)}{tool('italic', Italic, t('editor.italic'), true)}{tool('underline', Underline, t('editor.underline'), true)}{tool('strikeThrough', Strikethrough, t('editor.strike'), true)}
        <label className="toolbar-color" title={t('editor.color')}><Type size={16} /><input type="color" aria-label={t('editor.color')} disabled={disabled} onFocus={event => { const color = document.queryCommandValue('foreColor'); if (/^#[a-f\d]{6}$/i.test(color)) event.target.value = color; }} onChange={event => exec('foreColor', event.target.value)} /></label>
        <span className="toolbar-divider" />
        {tool('insertUnorderedList', List, t('editor.bulletList'), true)}{tool('insertOrderedList', ListOrdered, t('editor.orderedList'), true)}
        {tool('justifyLeft', AlignLeft, t('editor.alignLeft'), true)}{tool('justifyCenter', AlignCenter, t('editor.alignCenter'), true)}{tool('justifyRight', AlignRight, t('editor.alignRight'), true)}{tool('justifyFull', AlignJustify, t('editor.alignJustify'), true)}
        <button type="button" title={t('editor.quote')} aria-label={t('editor.quote')} disabled={disabled} onMouseDown={event => event.preventDefault()} onClick={() => exec('formatBlock', 'blockquote')}><Quote size={16} /></button>
        <button type="button" title={t('editor.insertLink')} aria-label={t('editor.insertLink')} aria-expanded={linkOpen} disabled={disabled} onMouseDown={event => event.preventDefault()} onClick={() => { setLinkText(window.getSelection()?.toString() ?? ''); setLinkOpen(value => !value); }}><Link2 size={16} /></button>
        {onPickImage && <button type="button" title={t('editor.insertImage')} aria-label={t('editor.insertImage')} disabled={disabled || picking} onMouseDown={event => event.preventDefault()} onClick={() => {
          void (async () => {
            setPicking(true);
            try {
              const picked = await onPickImage();
              // 文件对话框会移走焦点，先恢复光标再插入。
              if (picked) { restoreSelection(); exec('insertHTML', `<img src="${picked.dataUri}" alt="${escapeHtml(picked.name ?? '')}">`); }
            } finally { setPicking(false); }
          })();
        }}><ImageIcon size={16} /></button>}
        {tool('insertHorizontalRule', Minus, t('editor.insertHr'))}{tool('removeFormat', RemoveFormatting, t('editor.clearFormat'))}
        <span className="toolbar-divider" />{tool('undo', Undo2, t('editor.undo'))}{tool('redo', Redo2, t('editor.redo'))}
      </div>
      {linkOpen && <div className="toolbar-link" role="group" aria-label={t('editor.insertLink')} onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); setLinkOpen(false); restoreSelection(); } else if (event.key === 'Enter') { event.preventDefault(); insertLink(); } }}>
        <label>{t('editor.linkTextLabel')}<input aria-label={t('editor.linkTextAria')} value={linkText} onChange={event => setLinkText(event.target.value)} /></label>
        <label>{t('editor.linkUrlLabel')}<input aria-label={t('editor.linkUrlLabel')} value={linkUrl} placeholder="https://example.com" onChange={event => setLinkUrl(event.target.value)} autoFocus /></label>
        <div><button type="button" onClick={() => { setLinkOpen(false); restoreSelection(); }}>{t('editor.cancel')}</button><button type="button" onClick={insertLink} disabled={!SAFE_LINK.test(linkUrl.trim())}>{t('editor.insert')}</button></div>
      </div>}
    </div>
    <div className="compose-editor-frame">
      {empty && <div className="compose-placeholder" aria-hidden>{t('editor.placeholder')}</div>}
      <div ref={editorRef} className="compose-editor" contentEditable={!disabled} suppressContentEditableWarning role="textbox" aria-multiline aria-label={t('editor.bodyAria')} spellCheck onInput={emit} onFocus={syncStates}
        onPaste={event => {
          event.preventDefault();
          const html = event.clipboardData.getData('text/html');
          const text = event.clipboardData.getData('text/plain');
          // 剪贴板位图图片（截图 / 复制图片）→ data URI 内联插入；发送时由 outbox 转 cid
          // 内联件。svg 与超限图片明确提示，不静默丢弃。
          const images = [...event.clipboardData.items]
            .filter(item => item.kind === 'file' && /^image\/(png|jpeg|gif|webp|bmp)$/i.test(item.type))
            .map(item => item.getAsFile())
            .filter((file): file is File => file !== null);
          if (images.length) {
            let deferred = 0;
            for (const file of images) {
              if (file.size > MAX_INLINE_IMAGE) { deferred += 1; continue; }
              const reader = new FileReader();
              reader.onload = () => {
                if (typeof reader.result !== 'string') return;
                restoreSelection();
                exec('insertHTML', `<img src="${reader.result}" alt="">`);
              };
              reader.readAsDataURL(file);
            }
            if (deferred) setPasteNotice(t('editor.pasteImageTooLarge', { count: deferred }));
            return;
          }
          // HTML 粘贴里的远程 / svg 图片仍剥除并提示；位图 data:image 保留（cleanHtml）。
          let foreignImage = false;
          if (/<img\b/i.test(html)) {
            const probe = document.createElement('template');
            probe.innerHTML = html;
            foreignImage = [...probe.content.querySelectorAll('img')].some(img => !INLINE_IMAGE_SRC.test(img.getAttribute('src') ?? ''));
          }
          if ([...event.clipboardData.items].some(item => item.kind === 'file') || foreignImage) setPasteNotice(t('editor.pasteImageBlocked'));
          if (html) exec('insertHTML', cleanHtml(html));
          else if (text) exec('insertHTML', escapeHtml(text).replace(/\r?\n/g, '<br>'));
        }} />
    </div>
    {pasteNotice && <div className="compose-editor-notice" role="status">{pasteNotice}<button type="button" aria-label={t('editor.pasteNoticeClose')} onClick={() => setPasteNotice('')}><X size={14} /></button></div>}

  </div>;
}
