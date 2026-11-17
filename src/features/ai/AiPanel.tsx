import { AlignLeft, Languages, ListTodo, PenLine, Sparkles, X } from 'lucide-react';
import { Button } from '../../components/ui/Button';
import { useI18n } from '../../i18n/react';

export function AiPanel({ onClose }: { onClose: () => void }) {
  const t = useI18n();
  return <aside className="ai-panel" aria-label={t('ai.panelAria')}><header><span><Sparkles size={17} /> {t('ai.title')}</span><Button variant="ghost" iconOnly aria-label={t('ai.closeAria')} onClick={onClose}><X size={16} /></Button></header><div className="ai-panel-body"><div className="ai-mark"><Sparkles size={26} /></div><h2>{t('ai.heading')}</h2><p>{t('ai.intro')}</p><span className="label">{t('wizard.soonTag')}</span><div className="ai-actions">{[
    { Icon: AlignLeft, name: t('ai.summarize'), description: t('ai.summarizeDesc') },
    { Icon: PenLine, name: t('ai.draftReply'), description: t('ai.draftReplyDesc') },
    { Icon: ListTodo, name: t('ai.extractEvents'), description: t('ai.extractEventsDesc') },
    { Icon: Languages, name: t('ai.translate'), description: t('ai.translateDesc') },
  ].map(({ Icon, name, description }) => <div key={name}><Icon size={17} /><span><strong>{name}</strong><small>{description}</small></span></div>)}</div><p className="ai-privacy">{t('ai.privacy')}</p></div></aside>;
}
