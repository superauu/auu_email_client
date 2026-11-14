-- v11：撰写富文本。body_html 是草稿的可选 HTML 正文（编辑器产出，写入时服务端消毒）；
-- body_text 仍是纯文本替代版本，二者共同构成 multipart/alternative 的两个部分。
ALTER TABLE drafts ADD COLUMN body_html TEXT NOT NULL DEFAULT '';
