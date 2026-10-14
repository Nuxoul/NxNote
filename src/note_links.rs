//! Read-only wiki-link resolution and bounded, non-recursive note previews.
//!
//! The parent scanner removes `[[...]]`, `![[...]]` and aliases (`|label`) before
//! calling this module. The parent owns Ctrl-click dispatch, anchors, footnotes,
//! and rendering: preview text is untrusted Markdown, not HTML, and must not be
//! recursively expanded or allowed to load remote/local resources automatically.
//!
//! Identities are the existing (folder key, filename stem), not UUIDs. Rebuild a
//! context after Session::rename/create/delete; an old link does not follow a
//! rename. No registry, migration, link rewriting, or note creation occurs here.
//! Contexts built from a session snapshot only indexed, existing regular files.
//! Resolution is pure and case-sensitive. Preview rechecks the file on disk.
//!
//! Previews are immutable disk snapshots, never unsaved editor buffers. The
//! `current_text` parameter is deliberately unused: every direct self-embed,
//! including a section of the current note, is rejected. Nested wiki/embed syntax
//! remains literal, so indirect cycles do not trigger further reads.
//!
//! Storage's path helpers create directories and are intentionally not used.
//! The required config::data_dir() accessor itself may create the *data base*
//! directory; this module cannot suppress that existing API side effect. It
//! never creates the notes root, folders, or files, nor writes note contents.

use anyhow::{anyhow, bail, Context, Result};
use markdown::{mdast::Node, ParseOptions};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;

const MAX_READ_BYTES: usize = 1024 * 1024;
const MAX_EXCERPT_BYTES: usize = 6000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteId {
    pub folder_key: String,
    pub note_name: String,
}

#[derive(Clone, Debug)]
pub struct NoteLinkContext {
    pub current: NoteId,
    pub notes: Vec<NoteId>,
}

#[derive(Clone, Debug)]
pub struct NoteTarget {
    pub note: NoteId,
    pub heading: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NotePreview {
    pub title: String,
    pub text: String,
    pub target: NoteTarget,
}

impl NoteLinkContext {
    /// Snapshot the session index, including scratch and indexed default notes.
    /// Missing, invalid, escaped, and non-regular paths are not candidates.
    /// Indexing failure cannot be returned by this API: omitted candidates later
    /// resolve as missing; preview read failures are always explicit errors.
    pub fn from_session(session: &crate::session::Session) -> Self {
        let root = crate::config::data_dir().join("notes");
        let notes = session.all_notes().into_iter().map(|tab| NoteId {
            folder_key: tab.folder_key,
            note_name: tab.note_name,
        });
        Self {
            current: NoteId {
                folder_key: session.current.folder_key.clone(),
                note_name: session.current.note_name.clone(),
            },
            notes: existing_notes(&root, notes),
        }
    }

    /// Accept `title`, `folder_key/title`, either with `#heading`, or the
    /// qualified literal identity `nxnote:folder_key/note_name#heading`.
    /// No percent decoding, extension stripping, display-folder aliases, fuzzy
    /// matching, or fallback to an unindexed path is performed.
    pub fn resolve(&self, target: &str) -> Result<NoteTarget> {
        if target.chars().any(unsafe_control) {
            bail!("Note target contains control characters");
        }
        if target != target.trim() {
            bail!("Note target has leading or trailing whitespace");
        }
        let (name, heading) = match target.split_once('#') {
            Some((name, heading)) => {
                let heading = heading.trim();
                if heading.is_empty() {
                    bail!("Note heading is empty");
                }
                (name, Some(heading.to_string()))
            }
            None => (target, None),
        };
        let literal = name.starts_with("nxnote:");
        let name = name.strip_prefix("nxnote:").unwrap_or(name);
        let (folder, title) = match name.split_once('/') {
            Some((folder, title)) => {
                validate_component(folder)?;
                (Some(folder), title)
            }
            None if literal => bail!("nxnote: requires folder_key/note_name"),
            None => (None, name),
        };
        validate_component(title)?;
        if folder.is_some_and(|folder| folder.eq_ignore_ascii_case(".trash")) {
            bail!("The trash directory is not a note folder");
        }
        let mut candidates: Vec<&NoteId> = Vec::new();
        for note in &self.notes {
            if note.note_name == title
                && folder.map_or(true, |folder| note.folder_key == folder)
                && validate_id(note).is_ok()
                && !candidates.contains(&note)
            {
                candidates.push(note);
            }
        }
        if folder.is_none() {
            if let Some(local) = candidates
                .iter()
                .find(|note| note.folder_key == self.current.folder_key)
            {
                return Ok(NoteTarget { note: (*local).clone(), heading });
            }
        }
        match candidates.as_slice() {
            [] => bail!("Note not found in the existing-note index: {name}"),
            [note] => Ok(NoteTarget { note: (*note).clone(), heading }),
            _ => {
                let mut choices: Vec<String> = candidates.iter()
                    .map(|note| format!("{}/{}", note.folder_key, note.note_name))
                    .collect();
                choices.sort();
                bail!("Ambiguous note target {name}; specify a folder: {}", choices.join(", "))
            }
        }
    }
}

/// Read a bounded UTF-8 disk snapshot; self embeds and missing headings fail.
/// `current_text` is retained for the parent API but is never substituted for
/// disk text. No parser/scanner is invoked on nested wiki links or embeds.
pub fn preview(
    context: &NoteLinkContext,
    target: &str,
    current_text: &str,
) -> Result<NotePreview> {
    // Resolve/self-check before invoking config's directory-creating accessor.
    let resolved = context.resolve(target)?;
    if resolved.note == context.current {
        bail!("A note cannot embed itself");
    }
    preview_at_root(
        &crate::config::data_dir().join("notes"),
        context,
        resolved,
        current_text,
    )
}

fn preview_at_root(
    root: &Path,
    context: &NoteLinkContext,
    target: NoteTarget,
    _current_text: &str,
) -> Result<NotePreview> {
    if target.note == context.current {
        bail!("A note cannot embed itself");
    }
    let path = checked_note_path(root, &target.note)?;
    // Also reject alternate filenames pointing at the same canonical path.
    if checked_note_path(root, &context.current).ok().as_ref() == Some(&path) {
        bail!("A note cannot embed itself through an alias");
    }
    let text = read_snapshot(&path)?;
    let excerpt = match target.heading.as_deref() {
        Some(heading) => heading_section(&text, heading)?,
        None => text.as_str(),
    };
    let title = match &target.heading {
        Some(heading) => format!("{} — {heading}", target.note.note_name),
        None => target.note.note_name.clone(),
    };
    Ok(NotePreview { title, text: bounded_excerpt(excerpt), target })
}

fn existing_notes(root: &Path, notes: impl IntoIterator<Item = NoteId>) -> Vec<NoteId> {
    let mut existing: Vec<NoteId> = notes.into_iter()
        .filter(|note| checked_note_path(root, note).is_ok())
        .collect();
    existing.sort_by(|a, b| (&a.folder_key, &a.note_name).cmp(&(&b.folder_key, &b.note_name)));
    existing.dedup();
    existing
}

fn unsafe_control(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{feff}')
}

fn validate_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value != value.trim()
        || value == "."
        || value == ".."
        || value.ends_with('.')
        || value.chars().any(|c| unsafe_control(c)
            || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                | '%' | '#' | '[' | ']'))
    {
        bail!("Invalid note path component: {value:?}");
    }
    let stem = value.split('.').next().unwrap_or("").to_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
        || stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"));
    if device {
        bail!("Windows device names are not valid note paths: {value}");
    }
    Ok(())
}

fn validate_id(note: &NoteId) -> Result<()> {
    validate_component(&note.folder_key)?;
    validate_component(&note.note_name)?;
    if note.folder_key.eq_ignore_ascii_case(".trash") {
        bail!("The trash directory is not a note folder");
    }
    Ok(())
}

fn reject_redirect(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Cannot inspect note path {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("Symbolic links are not allowed in note paths: {}", path.display());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not just symlinks.
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("Reparse points are not allowed in note paths: {}", path.display());
        }
    }
    Ok(())
}

/// Only canonical, existing regular files beneath the canonical notes root.
/// Redirected folder/file components are rejected. This is a path-based check,
/// not a sandbox against a hostile process racing filesystem replacement.
fn checked_note_path(root: &Path, note: &NoteId) -> Result<PathBuf> {
    validate_id(note)?;
    let canonical_root = root.canonicalize().context("Notes root does not exist")?;
    if !fs::metadata(&canonical_root)?.is_dir() {
        bail!("Notes root is not a directory");
    }
    let folder = canonical_root.join(&note.folder_key);
    reject_redirect(&folder)?;
    let path = folder.join(format!("{}.md", note.note_name));
    reject_redirect(&path)?;
    let canonical_path = path.canonicalize().context("Note file does not exist")?;
    if !canonical_path.starts_with(&canonical_root) || !fs::metadata(&canonical_path)?.is_file() {
        bail!("Note is not a regular file beneath the notes root");
    }
    Ok(canonical_path)
}

fn read_snapshot(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("Cannot open note {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("Note is not a regular file");
    }
    if metadata.len() > MAX_READ_BYTES as u64 {
        bail!("Note exceeds the 1 MiB preview read limit");
    }
    let mut bytes = Vec::new();
    file.take((MAX_READ_BYTES + 1) as u64).read_to_end(&mut bytes)
        .with_context(|| format!("Cannot read note {}", path.display()))?;
    if bytes.len() > MAX_READ_BYTES {
        bail!("Note exceeds the 1 MiB preview read limit");
    }
    String::from_utf8(bytes).context("Note is not valid UTF-8")
}

fn inline_text(node: &Node, output: &mut String) {
    match node {
        Node::Text(text) => output.push_str(&text.value),
        Node::InlineCode(code) => output.push_str(&code.value),
        Node::Image(image) => output.push_str(&image.alt),
        Node::ImageReference(image) => output.push_str(&image.alt),
        Node::Break(_) => output.push(' '),
        _ => {
            if let Some(children) = node.children() {
                for child in children {
                    inline_text(child, output);
                }
            }
        }
    }
}

fn normalized_heading(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// A documented simple Unicode slug, not a promise of GitHub duplicate suffixes.
fn heading_slug(value: &str) -> String {
    normalized_heading(value).chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' { Some(c) }
            else if c.is_whitespace() { Some('-') }
            else { None }
        })
        .collect()
}

/// Markdown's parser recognizes ATX/Setext, inline formatting, and code fences.
/// Only document-level headings define sections; quoted/list headings do not.
/// Include the heading and children until the next same-or-higher-level heading.
/// Duplicate normalized heading/slug matches are an explicit ambiguity error.
fn heading_section<'a>(text: &'a str, requested: &str) -> Result<&'a str> {
    let root = markdown::to_mdast(text, &ParseOptions::gfm())
        .map_err(|error| anyhow!("Cannot parse preview Markdown: {error}"))?;
    let mut headings = Vec::new();
    for node in root.children().into_iter().flatten() {
        if let Node::Heading(heading) = node {
            if let Some(position) = node.position() {
                let mut label = String::new();
                inline_text(node, &mut label);
                headings.push((position.start.offset, heading.depth, label));
            }
        }
    }
    let requested = normalized_heading(requested);
    let matches: Vec<usize> = headings.iter().enumerate()
        .filter(|(_, (_, _, label))| normalized_heading(label) == requested || heading_slug(label) == requested)
        .map(|(index, _)| index)
        .collect();
    let index = match matches.as_slice() {
        [] => bail!("Heading not found: {requested}"),
        [index] => *index,
        _ => bail!("Ambiguous heading: {requested}"),
    };
    let (start, depth, _) = &headings[index];
    let end = headings[index + 1..].iter()
        .find(|(_, next_depth, _)| next_depth <= depth)
        .map_or(text.len(), |(start, _, _)| *start);
    text.get(*start..end).ok_or_else(|| anyhow!("Invalid Markdown heading source range"))
}

fn bounded_excerpt(text: &str) -> String {
    if text.len() <= MAX_EXCERPT_BYTES {
        return text.to_string();
    }
    let budget = MAX_EXCERPT_BYTES - '…'.len_utf8();
    let end = text.grapheme_indices(true)
        .take_while(|(start, grapheme)| start + grapheme.len() <= budget)
        .last().map_or(0, |(start, grapheme)| start + grapheme.len());
    format!("{}…", &text[..end])
}

/// Allow only explicit HTTP(S) and mailto URIs. No opening/network operation is
/// performed. Standalone `#anchors`, custom schemes, credentials, backslashes,
/// malformed escapes and raw/encoded controls are rejected. The parent handles
/// anchors and must call this validator before Ctrl-clicking an external link.
/// Percent-encoded percent signs are conservatively rejected (no double decode).
pub fn validate_external(target: &str) -> Result<String> {
    if target.is_empty() || target.chars().any(unsafe_external_char) {
        bail!("External link contains unsafe characters");
    }
    let decoded = decode_for_validation(target)?;
    if decoded.chars().any(|c| unsafe_control(c) || matches!(c, '\\' | '<' | '>' | '"' | '\'' | '`')) {
        bail!("External link contains encoded unsafe characters");
    }
    let (scheme, rest) = target.split_once(':').context("External link requires a scheme")?;
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => {
            let rest = rest.strip_prefix("//").context("HTTP(S) link requires // and a host")?;
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            validate_authority(authority)?;
        }
        "mailto" => {
            let address = rest.split('?').next().unwrap_or("");
            if address.is_empty() || address.contains('#') {
                bail!("mailto requires a recipient address");
            }
            // Decode addresses too, so escaped CR/LF or separators cannot bypass
            // the recipient checks. Header/query controls were checked above.
            let address = decode_for_validation(address)?;
            for recipient in address.split(',') {
                let (local, host) = recipient.rsplit_once('@').context("Invalid mailto recipient")?;
                if local.is_empty() || local.chars().any(unsafe_external_char)
                    || local.contains('@') || local.contains(['/', '?', '#', ':']) {
                    bail!("Invalid mailto recipient");
                }
                validate_host(host)?;
            }
        }
        _ => bail!("Only http, https, and mailto links are allowed"),
    }
    Ok(target.to_string())
}

fn unsafe_external_char(c: char) -> bool {
    unsafe_control(c) || c.is_whitespace() || matches!(c, '\\' | '<' | '>' | '"' | '\'' | '`')
}

fn decode_for_validation(value: &str) -> Result<String> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = bytes.next().and_then(|b| (b as char).to_digit(16));
            let lo = bytes.next().and_then(|b| (b as char).to_digit(16));
            let (Some(hi), Some(lo)) = (hi, lo) else { bail!("Malformed percent escape"); };
            let byte = (hi * 16 + lo) as u8;
            if byte == b'%' {
                bail!("Nested percent escapes are not allowed");
            }
            decoded.push(byte);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).context("External link escapes are not valid UTF-8")
}

fn validate_authority(authority: &str) -> Result<()> {
    if authority.is_empty() || authority.contains(['@', '%']) {
        bail!("HTTP(S) host is missing or contains credentials/escapes");
    }
    let port = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, suffix) = ipv6.split_once(']').context("Invalid IPv6 host")?;
        host.parse::<std::net::Ipv6Addr>().context("Invalid IPv6 host")?;
        if suffix.is_empty() { None } else { Some(suffix.strip_prefix(':').context("Invalid host suffix")?) }
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        validate_host(host)?;
        port
    };
    if let Some(port) = port {
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) || port.parse::<u16>().is_err() {
            bail!("Invalid HTTP(S) port");
        }
    }
    Ok(())
}

fn validate_host(host: &str) -> Result<()> {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty() || host.split('.').any(|label| {
        label.is_empty() || label.starts_with('-') || label.ends_with('-')
            || !label.chars().all(|c| c.is_alphanumeric() || c == '-')
    }) {
        bail!("Invalid external link host");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn id(folder: &str, name: &str) -> NoteId {
        NoteId { folder_key: folder.into(), note_name: name.into() }
    }

    fn context() -> NoteLinkContext {
        NoteLinkContext {
            current: id("a", "current"),
            notes: vec![id("a", "current"), id("a", "shared"), id("b", "shared"),
                id("b", "unique"), id("b", "duplicate"), id("c", "duplicate"),
                id("_global", "scratch"), id("a", "_default"), id("a", "中文🙂")],
        }
    }

    #[test]
    fn resolution_prefers_local_and_requires_unique_cross_folder() {
        let ctx = context();
        assert_eq!(ctx.resolve("shared").unwrap().note, id("a", "shared"));
        assert_eq!(ctx.resolve("unique").unwrap().note, id("b", "unique"));
        assert!(ctx.resolve("duplicate").unwrap_err().to_string().contains("Ambiguous"));
        assert_eq!(ctx.resolve("b/duplicate").unwrap().note, id("b", "duplicate"));
        let target = ctx.resolve("nxnote:b/unique#Some heading").unwrap();
        assert_eq!(target.note, id("b", "unique"));
        assert_eq!(target.heading.as_deref(), Some("Some heading"));
        assert!(ctx.resolve("nxnote:unique").is_err());
        assert_eq!(ctx.resolve("_global/scratch").unwrap().note, id("_global", "scratch"));
        assert_eq!(ctx.resolve("_default").unwrap().note, id("a", "_default"));
        assert_eq!(ctx.resolve("中文🙂").unwrap().note, id("a", "中文🙂"));
        let mut duplicate_index = ctx.clone();
        duplicate_index.notes.push(id("b", "unique"));
        assert!(duplicate_index.resolve("unique").is_ok());
    }

    #[test]
    fn invalid_and_unindexed_targets_fail_without_io() {
        for target in ["", "missing", "../shared", "a/../shared", "/shared",
            r"C:\shared", r"a\shared", "a//shared", "a/shared.", "a/shared ",
            "CON", "con.txt", "COM1", "LPT²", "a/shared\0", "a/shared\n",
            "a/%2e%2e", "shared|alias", "[[shared]]", "shared#", "#heading", ".trash/shared"] {
            assert!(context().resolve(target).is_err(), "{target:?}");
        }
        assert!(context().resolve("a/shared #heading").is_err());
        assert!(context().resolve(" shared").is_err());
        assert!(context().resolve("SHARED").is_err());
        assert!(preview(&context(), "current#Section", "unsaved").is_err());
    }

    #[test]
    fn headings_are_markdown_aware_and_sections_include_children() {
        let text = "# Intro\nlead\n\nSection **one**\n-------------\nbody\n### Child\nnested\n```md\n# Not a heading\n```\n## Next\nstop\n";
        let section = heading_section(text, "section-one").unwrap();
        assert!(section.starts_with("Section **one**\n---"));
        assert!(section.contains("### Child\nnested"));
        assert!(section.contains("# Not a heading"));
        assert!(!section.contains("## Next"));
        assert_eq!(heading_section(text, "SECTION ONE").unwrap(), section);
        assert!(heading_section(text, "Not a heading").is_err());
        assert!(heading_section("# Same\nx\n# Same\ny", "same").is_err());
        assert!(heading_section("> # Quoted\n\n    # Indented\n", "Quoted").is_err());
    }

    #[test]
    fn excerpts_never_split_utf8_or_graphemes() {
        let grapheme = "👩‍💻e\u{301}";
        let text = grapheme.repeat(2000);
        let excerpt = bounded_excerpt(&text);
        assert!(excerpt.len() <= MAX_EXCERPT_BYTES);
        assert!(excerpt.ends_with('…'));
        let prefix = excerpt.strip_suffix('…').unwrap();
        assert!(text.starts_with(prefix));
        assert!(text.grapheme_indices(true).any(|(offset, _)| offset == prefix.len()));
        assert_eq!(bounded_excerpt("short"), "short");
        assert_eq!(bounded_excerpt(&"x".repeat(MAX_EXCERPT_BYTES)).len(), MAX_EXCERPT_BYTES);
        assert_eq!(bounded_excerpt(&format!("a{}", "\u{301}".repeat(7000))), "…");
    }

    // Unique OS-temp fixtures; no config override, registry, or project writes.
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let mut serial = NEXT.fetch_add(1, Ordering::Relaxed);
            loop {
                let path = std::env::temp_dir().join(format!("nxnote-note-links-{}-{serial}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        serial = NEXT.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => panic!("Cannot create isolated fixture: {error}"),
                }
            }
        }
        fn write(&self, note: &NoteId, bytes: &[u8]) -> PathBuf {
            let folder = self.0.join(&note.folder_key);
            fs::create_dir_all(&folder).unwrap();
            let path = folder.join(format!("{}.md", note.note_name));
            fs::write(&path, bytes).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn disk_previews_are_literal_read_only_snapshots() {
        let fixture = Fixture::new();
        let ctx = context();
        let bytes = b"# Heading\nSaved ![[a/current]] and ![[b/unique]]\n## Child\nchild\n# End\n";
        let path = fixture.write(&id("b", "unique"), bytes);
        fixture.write(&ctx.current, b"unsaved counterpart on disk");
        let preview = preview_at_root(&fixture.0, &ctx, ctx.resolve("unique#heading").unwrap(), "UNSAVED").unwrap();
        assert_eq!(preview.title, "unique — heading");
        assert!(preview.text.contains("![[a/current]] and ![[b/unique]]"));
        assert!(!preview.text.contains("UNSAVED"));
        assert!(!preview.text.contains("# End"));
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(preview_at_root(&fixture.0, &ctx, ctx.resolve("current").unwrap(), "unsaved").is_err());
        assert!(preview_at_root(&fixture.0, &ctx, ctx.resolve("unique#missing").unwrap(), "").is_err());
    }

    #[test]
    fn indexed_files_are_filtered_and_read_failures_are_explicit() {
        let fixture = Fixture::new();
        let ctx = context();
        let scratch = id("_global", "scratch");
        let default = id("a", "_default");
        fixture.write(&scratch, b"");
        fixture.write(&default, b"default");
        let indexed = existing_notes(&fixture.0, vec![scratch.clone(), default.clone(), scratch.clone(), id("x", "missing"), id("..", "escape")]);
        assert_eq!(indexed, vec![scratch, default]);
        assert!(!fixture.0.join("x").exists());
        assert!(preview_at_root(&fixture.0, &ctx, ctx.resolve("unique").unwrap(), "").is_err());
        fixture.write(&id("b", "unique"), &[0xff, 0xfe]);
        assert!(preview_at_root(&fixture.0, &ctx, ctx.resolve("unique").unwrap(), "").unwrap_err().to_string().contains("UTF-8"));
        fixture.write(&id("b", "unique"), &vec![b'x'; MAX_READ_BYTES + 1]);
        assert!(preview_at_root(&fixture.0, &ctx, ctx.resolve("unique").unwrap(), "").unwrap_err().to_string().contains("1 MiB"));
        let path = fixture.write(&id("b", "unique"), b"valid");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(checked_note_path(&fixture.0, &id("b", "unique")).is_err());
        let absent = fixture.0.join("no-notes-root");
        assert!(checked_note_path(&absent, &id("a", "new")).is_err());
        assert!(!absent.exists());
    }

    #[test]
    fn filenames_are_identity_and_old_links_do_not_follow_renames() {
        let fixture = Fixture::new();
        let old = id("b", "unique");
        let new = id("b", "renamed");
        let old_path = fixture.write(&old, b"preserved contents");
        let new_path = fixture.0.join("b/renamed.md");
        fs::rename(&old_path, &new_path).unwrap();
        let stale = context();
        assert!(preview_at_root(&fixture.0, &stale, stale.resolve("unique").unwrap(), "").is_err());
        let refreshed = NoteLinkContext {
            current: stale.current,
            notes: existing_notes(&fixture.0, vec![new.clone()]),
        };
        assert!(refreshed.resolve("unique").is_err());
        assert_eq!(refreshed.resolve("nxnote:b/renamed").unwrap().note, new);
        assert_eq!(fs::read(&new_path).unwrap(), b"preserved contents");
    }

    #[cfg(unix)]
    #[test]
    fn redirected_files_and_folders_cannot_escape_root() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let outside_path = outside.write(&id("b", "unique"), b"private");
        fs::create_dir(fixture.0.join("b")).unwrap();
        symlink(&outside_path, fixture.0.join("b/unique.md")).unwrap();
        assert!(checked_note_path(&fixture.0, &id("b", "unique")).is_err());
        symlink(outside.0.join("b"), fixture.0.join("redirect")).unwrap();
        assert!(checked_note_path(&fixture.0, &id("redirect", "unique")).is_err());
    }

    #[test]
    fn external_allowlist_and_percent_defenses() {
        for target in ["https://example.com/path#heading", "HTTP://localhost:8080/a?x=1",
            "https://[::1]:443/", "https://example.com/%E4%B8%AD%E6%96%87",
            "mailto:a@example.com", "mailto:a@example.com,b@example.org?subject=Hello%20world"] {
            assert_eq!(validate_external(target).unwrap(), target);
        }
        for target in ["", "#heading", "javascript:alert(1)", "file:///tmp/note", "nxnote:a/note",
            "data:text/html,x", "https:example.com", "https:///path", "https://",
            "https://user:pass@example.com", "https://example.com:bad", "https://[bad]/",
            "https://example.com:65536", "https://ex%61mple.com", "https://example.com\\evil",
            " https://example.com", "https://example.com/a b", "https://example.com/%0d%0A",
            "https://example.com/%5c", "https://example.com/%250a", "https://example.com/%",
            "https://example.com/%zz", "https://example.com/%C2%85", "https://example.com/%FF",
            "https://example.com/%E2%80%AE", "https://example.com/\u{202e}", "mailto:",
            "mailto:not-an-address", "mailto:a@example.com?subject=x%0ABcc:evil@example.com"] {
            assert!(validate_external(target).is_err(), "{target:?}");
        }
    }
}
