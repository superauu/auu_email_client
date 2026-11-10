import type { MailAccount } from '../../domain/mail';

export function Avatar({ text, color = 'teal', small = false }: { text: string; color?: MailAccount['color']; small?: boolean }) {
  return <span className={`avatar tone-${color} ${small ? 'avatar-small' : ''}`} aria-hidden="true">{text.slice(0, 2)}</span>;
}
