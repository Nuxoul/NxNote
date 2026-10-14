//! CPU-only Markdown assets for GPUI image elements and native styled text.
//!
//! `render` accepts the *body* of a formula/diagram, not Markdown delimiters. It
//! never executes TeX, JavaScript, a browser, or an external program. The RaTeX
//! parser/layout/standalone-SVG pipeline and Mermaid renderer were API-checked
//! against Velotype's helpers (https://github.com/manyougz/velotype); this module
//! is an independent implementation, not copied source.
//!
//! Limitations: RaTeX implements a KaTeX-like math subset, not a TeX document
//! engine. User-defined macros, excessive nesting, embedded images/emoji and
//! unresolved text glyphs are rejected. Mermaid is deliberately limited to
//! flowchart/graph, sequence, class, state, ER, pie and mindmap diagrams supported
//! by mermaid-rs-renderer 0.3.0; browser HTML labels, interaction and full Mermaid
//! compatibility are not promised. PNGs use a fixed light palette and 2x raster
//! density (returned dimensions are logical pixels). Mermaid text uses installed
//! system fonts, so glyph availability and metrics are machine-dependent.
//!
//! Source, graph, SVG and raster budgets reduce accidental resource exhaustion;
//! they are not a process sandbox or a hard execution-time/total-memory bound for
//! third-party parsers/layout/font loading. Call `render` on a background worker.
//! One mutex serializes rendering in this process; atomic same-directory rename
//! publishes complete cache files across processes. Cache growth is not evicted
//! here. Only generated PNGs are written, never Markdown/note files.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, ensure, Context};
use resvg::{tiny_skia, usvg};
use sha2::{Digest, Sha256};
use syntect::easy::ScopeRangeIterator;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxDefinition, SyntaxSet};

pub enum RenderKind {
    Math { display: bool },
    Mermaid,
}

#[derive(Clone)]
pub struct RenderedAsset {
    pub path: PathBuf,
    /// Logical pixels; the PNG has twice this dimension.
    pub width: f32,
    pub height: f32,
}

const RENDER_VERSION: &str = concat!(
    "nxnote-markdown-v1/light/2x;ratex-parser-layout-svg=0.1.9;",
    "ratex-types-font-lexer=0.1.14;mermaid=0.3.0;resvg-usvg=0.45.1"
);
const MAX_MATH_BYTES: usize = 4 * 1024;
const MAX_MERMAID_BYTES: usize = 16 * 1024;
const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
const MAX_PNG_BYTES: u64 = 20 * 1024 * 1024;
const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u64 = 4 * 1024 * 1024; // 16 MiB RGBA main raster.
const DENSITY: f32 = 2.0;
static RENDER_LOCK: Mutex<()> = Mutex::new(());
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

/// Render and cache a native PNG, or return a contextual error for the UI.
///
/// Invalid/unsupported input is never replaced by raw source or an error SVG.
/// Cache hits are validated, including a bounded PNG decode; a corrupt cache
/// entry is reported explicitly (it can be removed to request regeneration).
/// The cache key covers exact source, kind, display style and renderer versions.
pub fn render(kind: &RenderKind, source: &str) -> anyhow::Result<RenderedAsset> {
    validate_source(kind, source)?;
    let _guard = RENDER_LOCK
        .lock()
        .map_err(|_| anyhow!("Markdown renderer mutex is poisoned"))?;
    let dir = crate::config::data_dir().join("cache").join("markdown-render");
    fs::create_dir_all(&dir)
        .with_context(|| format!("create Markdown render cache {}", dir.display()))?;
    let path = dir.join(format!("{}.png", cache_key(kind, source)));
    match fs::metadata(&path) {
        Ok(_) => return cached_asset(&path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("inspect Markdown render cache"),
    }
    // Convert ordinary dependency panics to diagnostics without poisoning our
    // mutex. Abort/stack overflow/OOM cannot be caught by catch_unwind.
    let (png, width, height) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let svg = generate_svg(kind, source)?;
        rasterize(&svg)
    }))
    .map_err(|_| anyhow!("native Markdown renderer panicked"))??;
    publish_png(&path, &png)?;
    // A second process may have won publication. Dimensions must describe the
    // file actually present, not our losing render's system-font metrics.
    let asset = cached_asset(&path)?;
    debug_assert!(width > 0 && height > 0);
    Ok(asset)
}

fn cache_key(kind: &RenderKind, source: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(RENDER_VERSION.as_bytes());
    hash.update([0]);
    hash.update(match kind {
        RenderKind::Math { display: false } => b"math-inline".as_slice(),
        RenderKind::Math { display: true } => b"math-display".as_slice(),
        RenderKind::Mermaid => b"mermaid".as_slice(),
    });
    hash.update([0]);
    hash.update(source.as_bytes());
    format!("{:x}", hash.finalize())
}

fn validate_source(kind: &RenderKind, source: &str) -> anyhow::Result<()> {
    ensure!(!source.trim().is_empty(), "empty Markdown render source");
    ensure!(!source.contains('\0'), "NUL in Markdown render source");
    let limit = match kind {
        RenderKind::Math { .. } => MAX_MATH_BYTES,
        RenderKind::Mermaid => MAX_MERMAID_BYTES,
    };
    ensure!(source.len() <= limit, "Markdown render source exceeds {limit} bytes");
    ensure!(source.lines().count() <= 256, "Markdown render source exceeds 256 lines");
    if matches!(kind, RenderKind::Math { .. }) {
        // Lexical preflight prevents deep user input before entering recursive
        // parser/layout code. Count commands too: implicit groups can nest even
        // without braces. This intentionally limits very elaborate formulae.
        let bytes = source.as_bytes();
        let mut i = 0;
        let mut depth = 0usize;
        let mut commands = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'%' => {
                    while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
                    continue;
                }
                b'\\' => {
                    i += 1;
                    let start = i;
                    while i < bytes.len() && bytes[i].is_ascii_alphabetic() { i += 1; }
                    commands += 1;
                    ensure!(commands <= 128, "formula exceeds 128 commands");
                    let command = &source[start..i];
                    ensure!(
                        !matches!(command, "def" | "gdef" | "edef" | "xdef" | "let" |
                            "futurelet" | "newcommand" | "renewcommand" | "providecommand" |
                            "global" | "expandafter" | "csname" | "loop"),
                        "user macro/expansion command \\{command} is not supported"
                    );
                    if start == i && i < bytes.len() {
                        // Escaped character (possibly non-ASCII): skip the full
                        // code point, preserving slice boundaries.
                        i += source[i..].chars().next().unwrap().len_utf8();
                    }
                    continue;
                }
                b'{' | b'[' | b'(' => {
                    depth += 1;
                    ensure!(depth <= 32, "formula nesting exceeds 32");
                }
                b'}' | b']' | b')' => depth = depth.saturating_sub(1),
                _ => {}
            }
            i += 1;
        }
    }
    Ok(())
}

fn generate_svg(kind: &RenderKind, source: &str) -> anyhow::Result<String> {
    let svg = match kind {
        RenderKind::Math { display } => {
            let parsed = ratex_parser::parse(source).map_err(|err| anyhow!("LaTeX parse: {err}"))?;
            let options = ratex_layout::LayoutOptions {
                style: if *display {
                    ratex_types::math_style::MathStyle::Display
                } else {
                    ratex_types::math_style::MathStyle::Text
                },
                ..Default::default()
            };
            let layout = ratex_layout::layout(&parsed, &options);
            let list = ratex_layout::to_display_list(&layout);
            ensure!(!list.items.is_empty(), "formula has no visible drawing commands");
            ensure!(list.items.len() <= 8192, "formula exceeds drawing budget");
            ensure!(list.width.is_finite() && list.height.is_finite() && list.depth.is_finite(),
                "non-finite formula dimensions");
            let svg = ratex_svg::render_to_svg(&list, &ratex_svg::SvgOptions {
                font_size: if *display { 22.0 } else { 18.0 },
                padding: 4.0,
                embed_glyphs: true,
                ..Default::default()
            });
            // RaTeX can fall back to <text> when a standalone glyph is absent.
            // Do not publish a deceptively incomplete formula in that case.
            ensure!(!svg.contains("<text"), "LaTeX glyph has no embedded outline");
            svg
        }
        RenderKind::Mermaid => {
            supported_mermaid_header(source)?;
            let parsed = mermaid_rs_renderer::parse_mermaid_strict(source)
                .map_err(|err| anyhow!("Mermaid parse: {err}"))?;
            ensure!(parsed.graph.nodes.len() <= 128 && parsed.graph.edges.len() <= 256
                && parsed.graph.mindmap.nodes.len() <= 128 && parsed.graph.pie_slices.len() <= 128
                && parsed.graph.sequence_participants.len() <= 128
                && parsed.graph.subgraphs.len() <= 32 && parsed.graph.sequence_frames.len() <= 32,
                "Mermaid exceeds node/edge/group budget (128 nodes, 256 edges, 32 groups)");
            let options = mermaid_rs_renderer::RenderOptions::default();
            let layout = mermaid_rs_renderer::compute_layout(
                &parsed.graph, &options.theme, &options.layout);
            let svg = mermaid_rs_renderer::render_svg(&layout, &options.theme, &options.layout);
            ensure!(!svg.contains("class=\"error-text\""), "Mermaid renderer produced an error diagram");
            svg
        }
    };
    ensure!(svg.len() <= MAX_SVG_BYTES, "generated SVG exceeds 4 MiB");
    // Reject rather than silently drop features that the offline rasterizer is
    // not allowed to resolve. No image resolver (including data URLs) is enabled.
    ensure!(!svg.contains("<image"), "embedded/external SVG images are unsupported");
    ensure!(!svg.contains("<foreignObject"), "HTML SVG labels are unsupported");
    Ok(svg)
}

fn supported_mermaid_header(source: &str) -> anyhow::Result<()> {
    // Frontmatter/init directives are intentionally not admitted: they can
    // override resource/layout settings behind the API's fixed policy.
    let first = source.lines().map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
        .ok_or_else(|| anyhow!("Mermaid diagram has no header"))?;
    let header = first.split(|ch: char| ch.is_whitespace() || ch == ';')
        .next().unwrap_or_default();
    ensure!(matches!(header, "flowchart" | "graph" | "sequenceDiagram" | "classDiagram" |
        "stateDiagram" | "stateDiagram-v2" | "erDiagram" | "pie" | "mindmap"),
        "unsupported Mermaid header {header:?}; expected flowchart, sequence, class, state, ER, pie or mindmap");
    ensure!(!source.contains("%%{"), "Mermaid init directives are unsupported");
    Ok(())
}

fn raster_dimensions(width: f32, height: f32) -> anyhow::Result<(u32, u32, f32)> {
    ensure!(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
        "invalid SVG dimensions");
    let w = f64::from(width);
    let h = f64::from(height);
    let scale = f64::from(DENSITY)
        .min(f64::from(MAX_SIDE) / w)
        .min(f64::from(MAX_SIDE) / h)
        .min((MAX_PIXELS as f64 / (w * h)).sqrt());
    let out_w = (w * scale).floor().max(1.0) as u32;
    let out_h = (h * scale).floor().max(1.0) as u32;
    ensure!(out_w <= MAX_SIDE && out_h <= MAX_SIDE && u64::from(out_w) * u64::from(out_h) <= MAX_PIXELS,
        "SVG raster exceeds size budget");
    Ok((out_w, out_h, scale as f32))
}

fn rasterize(svg: &str) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    ensure!(svg.len() <= MAX_SVG_BYTES, "SVG exceeds size budget");
    let mut options = usvg::Options::default();
    options.resources_dir = None;
    options.image_href_resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };
    // Fonts are loaded from OS-discovered font locations, never SVG URLs.
    // usvg's default font resolver selects only from this supplied database.
    if svg.contains("<text") {
        options.fontdb = FONTS.get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        }).clone();
        ensure!(options.fontdb.faces().next().is_some(), "no installed fonts for Mermaid text");
    }
    let tree = usvg::Tree::from_str(svg, &options).context("parse generated SVG for rasterization")?;
    let (width, height, scale) = raster_dimensions(tree.size().width(), tree.size().height())?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("allocate Markdown raster {width}x{height}"))?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let png = pixmap.encode_png().context("encode Markdown PNG")?;
    ensure!(png.len() as u64 <= MAX_PNG_BYTES, "PNG exceeds cache file budget");
    Ok((png, width, height))
}

fn cached_asset(path: &Path) -> anyhow::Result<RenderedAsset> {
    let file = File::open(path).with_context(|| format!("open render cache {}", path.display()))?;
    let meta = file.metadata().context("inspect cached PNG")?;
    ensure!(meta.is_file() && meta.len() <= MAX_PNG_BYTES, "invalid/oversized PNG cache entry");
    let mut header = [0u8; 24];
    let mut reader = BufReader::new(file);
    reader.read_exact(&mut header).context("read cached PNG header")?;
    ensure!(&header[..8] == b"\x89PNG\r\n\x1a\n" && &header[12..16] == b"IHDR",
        "invalid PNG cache header");
    let width = u32::from_be_bytes(header[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(header[20..24].try_into().unwrap());
    ensure!(width > 0 && height > 0 && width <= MAX_SIDE && height <= MAX_SIDE &&
        u64::from(width) * u64::from(height) <= MAX_PIXELS, "cached PNG dimensions exceed budget");
    let file = File::open(path).context("reopen cached PNG")?;
    let mut decoder = image::ImageReader::with_format(BufReader::new(file), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    decoder.limits(limits);
    let decoded = decoder.decode().context("corrupt Markdown PNG cache; remove entry to regenerate")?;
    ensure!(decoded.width() == width && decoded.height() == height, "cached PNG dimensions changed");
    Ok(RenderedAsset { path: path.to_owned(), width: width as f32 / DENSITY, height: height as f32 / DENSITY })
}

struct TemporaryFile(PathBuf);
impl Drop for TemporaryFile {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

fn publish_png(path: &Path, png: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("cache path has no parent"))?;
    let mut temporary = None;
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(".render-{}-{sequence}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => { temporary = Some((TemporaryFile(tmp), file)); break; }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err).context("create unique Markdown cache staging file"),
        }
    }
    let (temporary, mut file) = temporary.ok_or_else(|| anyhow!("cannot reserve cache staging file"))?;
    file.write_all(png).context("write Markdown PNG staging file")?;
    file.sync_all().context("flush Markdown PNG staging file")?;
    drop(file); // Windows rename requires closed handles.
    match fs::rename(&temporary.0, path) {
        Ok(()) => Ok(()),
        Err(err) if path.is_file() => {
            cached_asset(path).with_context(|| format!("cache publication race after {err}"))?;
            Ok(())
        }
        Err(err) => Err(err).context("atomically publish Markdown PNG cache"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeTone {
    Plain, Comment, Keyword, String, Number, Type, Function, Operator,
}

pub struct CodeSpan {
    pub source: Range<usize>,
    pub tone: CodeTone,
}

const MAX_CODE_BYTES: usize = 256 * 1024;
const MAX_CODE_LINE_BYTES: usize = 16 * 1024;
static SYNTAXES: OnceLock<Result<SyntaxSet, String>> = OnceLock::new();

/// Native grammar-based syntax coloring, not a compiler/semantic analyzer.
///
/// Shipped syntect grammars cover Rust, JS, Python, JSON, YAML, HTML, CSS,
/// shell, C/C++, Java, Go, SQL and Markdown. Small original embedded grammars
/// add TOML and TypeScript (TS-specific tokens plus the JavaScript grammar;
/// TSX/JSX semantics and template interpolation are grammar-dependent).
/// Parse state and scope stacks survive line boundaries, including multiline
/// strings/comments. Unknown languages, oversized input, or a grammar error
/// produce one explicit Plain span (this infallible API has no error channel).
/// Every nonempty input is covered exactly once, with UTF-8 byte boundaries;
/// the source, including CRLF and final newline, is never normalized.
pub fn highlight_code(source: &str, language: &str) -> Vec<CodeSpan> {
    if source.is_empty() { return Vec::new(); }
    if source.len() > MAX_CODE_BYTES || source.split_inclusive('\n').any(|line| line.len() > MAX_CODE_LINE_BYTES) {
        return plain_code(source);
    }
    let Some(extension) = language_extension(language) else { return plain_code(source); };
    let Ok(syntaxes) = SYNTAXES.get_or_init(make_syntaxes) else { return plain_code(source); };
    let Some(syntax) = syntaxes.find_syntax_by_extension(extension) else { return plain_code(source); };
    let mut parser = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut classes: HashMap<Scope, CodeTone> = HashMap::new();
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let Ok(ops) = parser.parse_line(line, syntaxes) else { return plain_code(source); };
        for (range, op) in ScopeRangeIterator::new(&ops, line) {
            if stack.apply(op).is_err() { return plain_code(source); }
            if range.is_empty() { continue; }
            if range.end > line.len() || !line.is_char_boundary(range.start) || !line.is_char_boundary(range.end) {
                return plain_code(source);
            }
            let mut tone = CodeTone::Plain;
            // Outer comment/string scopes take precedence over punctuation and
            // escapes inside them. Otherwise the innermost semantic scope wins.
            for scope in stack.as_slice() {
                let class = *classes.entry(*scope).or_insert_with(|| scope_tone(&scope.to_string()));
                if class == CodeTone::Comment || class == CodeTone::String { tone = class; break; }
                if class != CodeTone::Plain { tone = class; }
            }
            push_span(&mut spans, offset + range.start..offset + range.end, tone);
        }
        offset += line.len();
    }
    if !valid_coverage(source, &spans) { return plain_code(source); }
    spans
}

fn plain_code(source: &str) -> Vec<CodeSpan> {
    if source.is_empty() { Vec::new() }
    else { vec![CodeSpan { source: 0..source.len(), tone: CodeTone::Plain }] }
}

fn push_span(spans: &mut Vec<CodeSpan>, source: Range<usize>, tone: CodeTone) {
    if let Some(last) = spans.last_mut() {
        if last.tone == tone && last.source.end == source.start { last.source.end = source.end; return; }
    }
    spans.push(CodeSpan { source, tone });
}

fn valid_coverage(source: &str, spans: &[CodeSpan]) -> bool {
    let mut end = 0;
    for span in spans {
        if span.source.start != end || span.source.end <= end || span.source.end > source.len()
            || !source.is_char_boundary(span.source.start) || !source.is_char_boundary(span.source.end) { return false; }
        end = span.source.end;
    }
    end == source.len()
}

fn scope_tone(scope: &str) -> CodeTone {
    if scope.starts_with("comment") { CodeTone::Comment }
    else if scope.starts_with("string") { CodeTone::String }
    else if scope.starts_with("constant.numeric") { CodeTone::Number }
    else if scope.starts_with("keyword.operator") { CodeTone::Operator }
    else if scope.starts_with("keyword") || scope.starts_with("storage.modifier")
        || scope.starts_with("constant.language") { CodeTone::Keyword }
    else if scope.starts_with("storage.type") || scope.starts_with("entity.name.type")
        || scope.starts_with("entity.name.class") || scope.starts_with("support.type")
        || scope.starts_with("support.class") { CodeTone::Type }
    else if scope.starts_with("entity.name.function") || scope.starts_with("support.function") { CodeTone::Function }
    else { CodeTone::Plain }
}

fn language_extension(language: &str) -> Option<&'static str> {
    match language.split_whitespace().next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "rust" | "rs" => Some("rs"),
        "javascript" | "js" | "jsx" => Some("js"),
        "typescript" | "ts" | "tsx" => Some("ts"),
        "python" | "py" => Some("py"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "bash" | "sh" | "shell" | "zsh" => Some("sh"),
        "c" | "h" => Some("c"),
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => Some("cpp"),
        "java" => Some("java"),
        "go" | "golang" => Some("go"),
        "sql" => Some("sql"),
        "markdown" | "md" => Some("md"),
        _ => None,
    }
}

fn make_syntaxes() -> Result<SyntaxSet, String> {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    for grammar in [TYPESCRIPT_GRAMMAR, TOML_GRAMMAR] {
        builder.add(SyntaxDefinition::load_from_str(grammar, true, None).map_err(|err| err.to_string())?);
    }
    Ok(builder.build())
}

// Original, intentionally lexical extensions; not third-party grammar copies.
const TYPESCRIPT_GRAMMAR: &str = r#"%YAML 1.2
---
name: NxNote TypeScript
file_extensions: [ts, tsx]
scope: source.ts
contexts:
  main:
    - match: '\b(interface|type|namespace|declare|abstract|implements|readonly|keyof|infer|is|asserts|satisfies)\b'
      scope: keyword.declaration.ts
    - match: '\b(string|number|boolean|any|unknown|never|void|bigint|symbol)\b'
      scope: storage.type.ts
    - include: scope:source.js
"#;

const TOML_GRAMMAR: &str = r#"%YAML 1.2
---
name: NxNote TOML
file_extensions: [toml]
scope: source.toml
contexts:
  main:
    - match: '#.*$'
      scope: comment.line.toml
    - match: '"""'
      push: multiline-basic
    - match: "'''"
      push: multiline-literal
    - match: '"'
      push: basic
    - match: "'"
      push: literal
    - match: '\b(true|false)\b'
      scope: constant.language.toml
    - match: '[+-]?(?:\b(?:0x[0-9A-Fa-f_]+|0o[0-7_]+|0b[01_]+|[0-9][0-9_]*(?:\.[0-9_]+)?(?:[eE][+-]?[0-9_]+)?)\b|\b(?:inf|nan)\b)'
      scope: constant.numeric.toml
    - match: '[=.,\[\]{}]'
      scope: keyword.operator.toml
  basic:
    - meta_scope: string.quoted.double.toml
    - match: '\\.'
      scope: constant.character.escape.toml
    - match: '"|\n'
      pop: true
  literal:
    - meta_scope: string.quoted.single.toml
    - match: "'|\n"
      pop: true
  multiline-basic:
    - meta_scope: string.quoted.double.toml
    - match: '\\(?:.|\n)'
      scope: constant.character.escape.toml
    - match: '"""'
      pop: true
  multiline-literal:
    - meta_scope: string.quoted.single.toml
    - match: "'''"
      pop: true
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_and_all_language_aliases() {
        for language in ["rust", "js", "ts", "python", "json", "yaml", "toml", "html", "css",
            "shell", "c", "cpp", "java", "go", "sql", "markdown", "unknown"] {
            let source = "/* α */\r\nlet café = \"你好\";\n42\n";
            let spans = highlight_code(source, language);
            assert!(valid_coverage(source, &spans), "{language}");
        }
        assert!(highlight_code("", "rust").is_empty());
        let spans = highlight_code("α\n", "not-a-language");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].tone, CodeTone::Plain);
    }

    #[test]
    fn every_requested_language_has_a_grammar() {
        let set = make_syntaxes().expect("embedded grammar YAML");
        for language in ["rust", "js", "ts", "python", "json", "yaml", "toml", "html", "css",
            "shell", "c", "cpp", "java", "go", "sql", "markdown"] {
            assert!(set.find_syntax_by_extension(language_extension(language).unwrap()).is_some(), "{language}");
        }
    }

    fn tone_at(source: &str, language: &str, needle: &str) -> CodeTone {
        let position = source.find(needle).unwrap();
        highlight_code(source, language).into_iter()
            .find(|span| span.source.contains(&position)).unwrap().tone
    }

    #[test]
    fn multiline_state_and_utf8_are_preserved() {
        let rust = "/* comment\n世界 */\nlet x = r#\"first\nsecond\"#;";
        assert_eq!(tone_at(rust, "rust", "世界"), CodeTone::Comment);
        assert_eq!(tone_at(rust, "rust", "second"), CodeTone::String);
        let python = "x = '''first\n世界'''\n# end";
        assert_eq!(tone_at(python, "python", "世界"), CodeTone::String);
        let toml = "x = \"\"\"first\n世界\"\"\"\ny = true # comment";
        assert_eq!(tone_at(toml, "toml", "世界"), CodeTone::String);
        assert_eq!(tone_at(toml, "toml", "comment"), CodeTone::Comment);
        let js = "/* first\n世界 */\nconst x = `first\nsecond`;";
        assert_eq!(tone_at(js, "js", "世界"), CodeTone::Comment);
        assert_eq!(tone_at(js, "ts", "second"), CodeTone::String);
    }

    #[test]
    fn oversized_code_is_bounded_plain_text() {
        let source = "x".repeat(MAX_CODE_BYTES + 1);
        let spans = highlight_code(&source, "rust");
        assert_eq!(spans.len(), 1);
        assert!(valid_coverage(&source, &spans));
    }

    #[test]
    fn cache_keys_separate_kind_style_and_exact_source() {
        let inline = RenderKind::Math { display: false };
        let display = RenderKind::Math { display: true };
        assert_eq!(cache_key(&inline, "x"), cache_key(&inline, "x"));
        assert_ne!(cache_key(&inline, "x"), cache_key(&display, "x"));
        assert_ne!(cache_key(&display, "x"), cache_key(&RenderKind::Mermaid, "x"));
        assert_ne!(cache_key(&inline, "x"), cache_key(&inline, "x "));
    }

    #[test]
    fn real_formula_and_diagram_rasterize_without_cache_writes() {
        for kind in [RenderKind::Math { display: false }, RenderKind::Math { display: true }] {
            let svg = generate_svg(&kind, r"\frac{1}{2}+x^2").expect("formula SVG");
            assert!(svg.contains("<path"));
            let (png, width, height) = rasterize(&svg).expect("formula PNG");
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            assert!(width > 8 && height > 8);
        }
        let svg = generate_svg(&RenderKind::Mermaid, "flowchart LR\n A[Start] --> B[End]").expect("diagram SVG");
        let (png, width, height) = rasterize(&svg).expect("diagram PNG");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(width > 8 && height > 8);
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert!(decoded.pixels().any(|pixel| pixel.0 != [255, 255, 255, 255]));
    }

    #[test]
    fn errors_and_resource_limits_are_explicit() {
        assert!(generate_svg(&RenderKind::Math { display: true }, r"\frac{a}").is_err());
        assert!(supported_mermaid_header("not-a-diagram").is_err());
        assert!(supported_mermaid_header("%%{init: {}}%%\nflowchart LR\nA-->B").is_err());
        assert!(validate_source(&RenderKind::Math { display: false }, r"\def\x{\x}\x").is_err());
        assert!(validate_source(&RenderKind::Math { display: true }, &"{".repeat(40)).is_err());
        assert!(validate_source(&RenderKind::Mermaid, &"x".repeat(MAX_MERMAID_BYTES + 1)).is_err());
        assert!(raster_dimensions(f32::NAN, 1.0).is_err());
        let (width, height, _) = raster_dimensions(100_000.0, 100_000.0).unwrap();
        assert!(width <= MAX_SIDE && height <= MAX_SIDE);
        assert!(u64::from(width) * u64::from(height) <= MAX_PIXELS);
    }
}
