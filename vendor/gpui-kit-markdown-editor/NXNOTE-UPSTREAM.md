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

NxNote compact-chrome pass: gutter paints the document background instead of a
contrasting secondary lane; the one-device-pixel divider uses 55% border alpha,
inactive numbers 80% muted alpha and the active number the foreground. Fixed
allocation, full-height coverage, native positions and editing semantics remain.
This is a host-specific presentation choice, not code copied from Zed. The unused
style parameter is marked intentionally unused; signature/callers are unchanged.

Insertion caret geometry now uses the rendered leaf line height instead of font
ascent + descent. Its top is the native line origin, without glyph-height centering;
painting, reveal and IME bounds share FrameLayout::caret_bounds. Existing device
pixel snapping remains. This supports the host's 0.5–2.5 line-height range without
introducing a separate caret/input geometry model.

Paragraph spacing is now a separate host setting. Include the resolved paragraph
gap in the culling geometry key, so changing only that setting drops cached block
heights and schedules fresh measurement without a document/history mutation.
The host defaults the extra gap to zero; font line-height remains independent.
