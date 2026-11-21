import { useEffect, useRef, type ReactNode } from 'react';

export function SettingsPageHeading({ title, description, actions, focus = false, level = 'h3' }: {
  title: string;
  description?: string;
  actions?: ReactNode;
  focus?: boolean;
  level?: 'h3' | 'h4';
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  const Heading = level;
  useEffect(() => { if (focus) heading.current?.focus(); }, [focus, title]);

  return <header className="settings-page-heading">
    <div>
      <Heading ref={heading} tabIndex={focus ? -1 : undefined}>{title}</Heading>
      {description && <p>{description}</p>}
    </div>
    {actions && <div className="settings-actions">{actions}</div>}
  </header>;
}
