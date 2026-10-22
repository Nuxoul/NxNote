# Vendored upstream

Source: https://github.com/wabzqem/gpui-kit-markdown-editor
Commit: 31608fa74bdbeca79d9901bf2daa48ea7ecf1ecc
Version: 0.1.0
License: Apache-2.0 (LICENSE-APACHE retained)

Selected by the user as NxNote's default editor after the A/B comparison. Application integration is in src/note_editor.rs. Upstream source is not an NxNote-authored editing engine. Local changes are documented below.

Local extension: `BlockEditorState::apply_markdown_paste` resolves a host-admitted image as Markdown through the existing request validation and model paste path, without re-reading the clipboard or triggering a second paste request.

Stable-toolchain compatibility: replace three unstable `usize::isolate_lowest_one()` calls in `cull.rs` with equivalent `i & i.wrapping_neg()` Fenwick low-bit operations.

Optional presentation extension: `BlockEditor::line_numbers(true)` reserves a narrow
left gutter inside the native input root. A revision-cached full-document index
counts logical rendered text lines, not serialized Markdown syntax; soft wraps
keep one number and same-row table cells share numbers. Paint uses current native
leaf geometry, clipping, and caret state, without another input/scroll owner.

Gutter presentation: regular Consolas digits, pixel-snapped right alignment and
active-number emphasis only. The total layout allocation is a fixed 64px including
a separate 16px handle strip; digit-count changes never resize it. Background uses
native root bounds (including vertical insets), reaches the left viewport edge,
and has a one-device-pixel right divider. Paragraph/editing semantics are unchanged.

Host line-height refinement is respected across headings, code blocks and table
cells as well as inherited body text. Existing TextViewStyle refinements are
preserved; typography-based culling invalidation and native gutter geometry are
reused. No document, input, history or paragraph-gap rules are changed.

`BlockEditor::text_style` accepts a host TextViewStyle without altering the model.
Selection decorations split text/backgrounds from inline-atom overlays: text keeps
the host selection color, atom overlays use 40% alpha to leave images visible.
The gutter now reads semantic surface/border/text roles from the shared theme.
