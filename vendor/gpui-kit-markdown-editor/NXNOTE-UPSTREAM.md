# Vendored upstream

Source: https://github.com/wabzqem/gpui-kit-markdown-editor
Commit: 31608fa74bdbeca79d9901bf2daa48ea7ecf1ecc
Version: 0.1.0
License: Apache-2.0 (LICENSE-APACHE retained)

Selected by the user as NxNote's default editor after the A/B comparison. Application integration is in src/note_editor.rs. Upstream source is not an NxNote-authored editing engine. Local changes are documented below.

Local extension: `BlockEditorState::apply_markdown_paste` resolves a host-admitted image as Markdown through the existing request validation and model paste path, without re-reading the clipboard or triggering a second paste request.

Stable-toolchain compatibility: replace three unstable `usize::isolate_lowest_one()` calls in `cull.rs` with equivalent `i & i.wrapping_neg()` Fenwick low-bit operations.
