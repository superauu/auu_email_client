/**
 * 附件预览能力判定（与 Rust mail_attachments 规则镜像）：
 * 位图图片（png/jpeg/gif/webp/bmp）与文本类（text/*、json、xml、常见文本扩展名）
 * 可内联预览；其余类型（含 svg——收件端普遍拦截）只提供另存为。
 */
const IMAGE_MIME = ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/bmp'];
const TEXT_SUFFIX = ['.txt', '.md', '.csv', '.log', '.json'];

export function isPreviewableAttachment(mediaType: string, name: string): boolean {
  const lower = mediaType.toLowerCase();
  if (IMAGE_MIME.includes(lower)) return true;
  const fileName = name.toLowerCase();
  return lower.startsWith('text/')
    || lower === 'application/json'
    || lower === 'application/xml'
    || TEXT_SUFFIX.some(suffix => fileName.endsWith(suffix));
}

/** mail_preview_attachment 的返回形状（kind 'image' 有 dataUri；'text' 有 text）。 */
export interface AttachmentPreview {
  kind: 'image' | 'text';
  dataUri: string | null;
  text: string | null;
  truncated: boolean;
}
