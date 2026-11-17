import { useId, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { isRecipientAddress, splitRecipients } from '../../domain/recipients';
import { useI18n } from '../../i18n/react';

export interface RecipientSuggestion { name: string; email: string }

export function RecipientField({ label, value, onChange, suggestions, disabled, autoFocus, children }: {
  label: string; value: string; onChange: (value: string) => void;
  suggestions: RecipientSuggestion[]; disabled?: boolean; autoFocus?: boolean; children?: React.ReactNode;
}) {
  const t = useI18n();
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const [tokens, setTokens] = useState(() => splitRecipients(value));
  const [pending, setPending] = useState('');
  const [focused, setFocused] = useState(false);
  const [highlight, setHighlight] = useState(-1);
  const [limitError, setLimitError] = useState(false);
  const matches = focused && pending.trim() ? suggestions.filter(item =>
    !tokens.some(token => token.toLowerCase() === item.email.toLowerCase()) &&
    `${item.name} ${item.email}`.toLowerCase().includes(pending.trim().toLowerCase()),
  ).slice(0, 5) : [];
  const invalid = tokens.some(token => !isRecipientAddress(token));

  function publish(next: string[], text: string) {
    setTokens(next); setPending(text); setHighlight(-1); setLimitError(false);
    onChange([...next, ...(text ? [text] : [])].join(', '));
  }
  function commit(text = pending) {
    const next = [...new Map([...tokens, ...splitRecipients(text)].map(token => [token.toLowerCase(), token])).values()];
    if (next.join(', ').length > 1000) { setLimitError(true); return; }
    publish(next, '');
  }
  function edit(index: number) {
    // Keep any partly typed address when editing a chip.
    if (pending) commit();
    else { publish(tokens.filter((_, i) => i !== index), tokens[index]); input.current?.focus(); }
  }

  return <div className="compose-address-row">
    <label htmlFor={id}>{label}</label>
    <div className="recipient-field">
      <div className="recipient-input-wrap">
        {tokens.map((token, index) => <span className={`recipient-chip${isRecipientAddress(token) ? '' : ' is-invalid'}`} key={`${index}-${token}`}>
          <button type="button" className="recipient-chip-edit" disabled={disabled} onClick={() => edit(index)} title={t('recipients.chipEditTitle', { address: token })}>{token}</button>
          <button type="button" disabled={disabled} aria-label={t('recipients.removeAria', { field: label, address: token })} onClick={() => { publish(tokens.filter((_, i) => i !== index), pending); input.current?.focus(); }}><X size={12} /></button>
        </span>)}
        <input ref={input} id={id} value={pending} role="combobox" aria-autocomplete="list" aria-expanded={matches.length > 0} aria-controls={matches.length ? `${id}-list` : undefined} aria-activedescendant={highlight >= 0 && matches[highlight] ? `${id}-${highlight}` : undefined} aria-invalid={invalid} aria-describedby={invalid ? `${id}-error` : undefined}
          placeholder={tokens.length ? '' : t('recipients.placeholder')} autoFocus={autoFocus} autoComplete="off" disabled={disabled}
          maxLength={Math.max(0, 1000 - tokens.join(', ').length - (tokens.length ? 2 : 0))}
          onFocus={() => setFocused(true)} onBlur={() => { setFocused(false); commit(); }}
          onChange={event => { setFocused(true); const text = event.target.value; if (/[,;；，\n]/.test(text) && !text.includes('"')) commit(text); else publish(tokens, text); }}
          onPaste={event => { const text = event.clipboardData.getData('text'); if (/[,;；，\n<>]/.test(text)) { event.preventDefault(); commit(pending + text); } }}
          onKeyDown={event => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === 'Escape' && matches.length) { event.preventDefault(); event.stopPropagation(); setFocused(false); }
            else if (event.key === 'ArrowDown' && matches.length) { event.preventDefault(); setHighlight(index => (index + 1) % matches.length); }
            else if (event.key === 'ArrowUp' && matches.length) { event.preventDefault(); setHighlight(index => (index <= 0 ? matches.length : index) - 1); }
            else if (event.key === 'Enter') { event.preventDefault(); commit(matches[highlight]?.email ?? pending); }
            else if (event.key === 'Tab') commit();
            else if (event.key === 'Backspace' && !pending && tokens.length) { event.preventDefault(); edit(tokens.length - 1); }
          }} />
      </div>
      {invalid && <span className="recipient-error" id={`${id}-error`}>{t('recipients.invalidFormat')}</span>}
      {limitError && <span className="recipient-error" role="alert">{t('recipients.limitExceeded')}</span>}
      {matches.length > 0 && <div className="recipient-suggestions" id={`${id}-list`} role="listbox" aria-label={t('recipients.suggestionsAria', { field: label })}>
        {matches.map((item, index) => <button type="button" key={item.email} role="option" id={`${id}-${index}`} aria-selected={index === highlight} onMouseDown={event => event.preventDefault()} onClick={() => { commit(item.email); input.current?.focus(); }}><span>{item.name || item.email}</span><small>{item.email}</small></button>)}
      </div>}
    </div>
    {children}
  </div>;
}
