import { useEffect, useRef } from 'react';
import { Button } from './Button';
import { useI18n } from '../../i18n/react';

export function InlineConfirmation({ message, confirmLabel, cancelLabel, onConfirm, onCancel }: {
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const t = useI18n();
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => { cancel.current?.focus(); }, []);

  return <div className="inline-confirmation">
    <p role="status">{message}</p>
    <div>
      <Button ref={cancel} onClick={onCancel}>{cancelLabel ?? t('editor.cancel')}</Button>
      <Button className="danger-button" onClick={onConfirm}>{confirmLabel}</Button>
    </div>
  </div>;
}
