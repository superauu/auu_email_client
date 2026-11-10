import * as Primitive from '@radix-ui/react-dialog';
import { X } from 'lucide-react';
import { useRef, type ReactNode } from 'react';
import { Button } from './Button';

export function Dialog({ open, onOpenChange, title, description, children, className = '' }: {
  open: boolean; onOpenChange: (open: boolean) => void; title: string; description: string; children: ReactNode; className?: string;
}) {
  const returnFocus = useRef<HTMLElement | null>(null);
  return <Primitive.Root open={open} onOpenChange={onOpenChange}>
    <Primitive.Portal>
      <Primitive.Overlay className="dialog-overlay" />
      <Primitive.Content className={`dialog-content ${className}`} onOpenAutoFocus={() => { returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; }} onCloseAutoFocus={event => { event.preventDefault(); returnFocus.current?.focus(); }}>
        <header className="dialog-header">
          <div><Primitive.Title>{title}</Primitive.Title><Primitive.Description>{description}</Primitive.Description></div>
          <Primitive.Close asChild><Button variant="ghost" iconOnly aria-label="关闭对话框"><X size={18} /></Button></Primitive.Close>
        </header>
        {children}
      </Primitive.Content>
    </Primitive.Portal>
  </Primitive.Root>;
}
