import { AlignLeft, Languages, ListTodo, PenLine, Sparkles, X } from 'lucide-react';
import { Button } from '../../components/ui/Button';

export function AiPanel({ onClose }: { onClose: () => void }) {
  return <aside className="ai-panel" aria-label="AI 助手"><header><span><Sparkles size={17} /> AI 助手</span><Button variant="ghost" iconOnly aria-label="关闭 AI 助手" onClick={onClose}><X size={16} /></Button></header><div className="ai-panel-body"><div className="ai-mark"><Sparkles size={26} /></div><h2>给思考，一点助力</h2><p>摘要、回复和待办，让每封邮件更容易处理。</p><span className="label">即将支持</span><div className="ai-actions">{[
    { Icon: AlignLeft, name: '概括邮件内容', description: '快速掌握重点' },
    { Icon: PenLine, name: '起草回复', description: '把想法变成文字' },
    { Icon: ListTodo, name: '提取事件与待办', description: '让重要安排不遗漏' },
    { Icon: Languages, name: '翻译邮件', description: '让语言不再是距离' },
  ].map(({ Icon, name, description }) => <div key={name}><Icon size={17} /><span><strong>{name}</strong><small>{description}</small></span></div>)}</div><p className="ai-privacy">当前未启用 AI 服务，邮件内容不会发送给模型。生成的回复将作为草稿，由你确认后发送。</p></div></aside>;
}
