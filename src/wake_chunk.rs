//! Wake-ritual chunking primitives.
//!
//! Splits over-threshold bloom content into presentation-layer chunks so the
//! wake ritual survives the ~30KB Bash output ceiling. **No schema changes,
//! no persisted plan** — chunking is a pure deterministic function of
//! `(content, threshold, boundary rules)`. See `mx-211-wake-chunking-design.md`
//! for the full design and rationale.
//!
//! This module is standalone (PR 1): `compute_chunks`, `extract_salient_phrase`,
//! and `compare_phrase` are pure functions. No `wake_ritual` or `surreal_db`
//! integration lives here.
//!
//! ## Key guarantees
//!
//! - **Lossless reconstitution**: `chunks.concat() == original`
//!   (property-tested — Risk 2 in the design).
//! - **UTF-8 safety**: every boundary lands on a char boundary.
//! - **Code-block integrity**: fenced ``` blocks are never split mid-block.
//! - **Never-empty phrase**: `extract_salient_phrase` is a total function.
//! - **Determinism**: same inputs → same output, always.

use std::env;

/// Environment variable override for the chunking threshold.
///
/// Wonka measured the Bash output ceiling empirically at ~30KB on 2026-03-25
/// (fettle#13). We chunk at 28KB to keep headroom under that wall.
pub const DEFAULT_CHUNK_THRESHOLD: usize = 28_000;

/// Env var that overrides `DEFAULT_CHUNK_THRESHOLD`. Parse failures fall back
/// to the default silently — operators get observability via chunk-size logs.
pub const CHUNK_THRESHOLD_ENV: &str = "MX_WAKE_CHUNK_BYTES";

/// Resolve the active chunking threshold. Reads `MX_WAKE_CHUNK_BYTES` lazily
/// on every call so env changes take effect mid-process (useful for tests
/// and per-session overrides).
pub fn chunk_threshold() -> usize {
    match env::var(CHUNK_THRESHOLD_ENV) {
        Ok(v) => v
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_CHUNK_THRESHOLD),
        Err(_) => DEFAULT_CHUNK_THRESHOLD,
    }
}

/// A deterministic chunk plan over a specific `&str`.
///
/// The plan does not own the content; callers slice the original content with
/// `chunk()` / `chunk_at()`. This keeps the plan cheap to re-compute on every
/// `respond`/`skip` call (pure-runtime-projection).
///
/// `boundaries[i]` is the byte offset where chunk `i+1` begins (i.e. chunk 0
/// is `[0, boundaries[0])`, chunk 1 is `[boundaries[0], boundaries[1])`, etc).
/// Guarantees:
///
/// - `boundaries.len() + 1 == total as usize`
/// - boundaries are strictly increasing
/// - every boundary falls on a UTF-8 char boundary of the source content
/// - concatenating all chunks reproduces the original content exactly
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPlan {
    /// Total number of chunks. 1 for non-chunked (under-threshold) content.
    ///
    /// Widened from `u8` to `u16` after Diffi's review of mx#212: at the
    /// default 28KB threshold a bloom would need ~7MB of content to overflow
    /// u8, but `MX_WAKE_CHUNK_BYTES=50` on a 15KB bloom reproduced silent
    /// saturation at 255. `plan.total` is consumed by progress display and
    /// chunk indexing in PR 2, so a saturated `total` produced wrong UX and
    /// potentially dropped chunks on the last-chunk branch.
    pub total: u16,
    /// Byte offsets where each chunk after the first begins. `len() == total - 1`.
    pub boundaries: Vec<usize>,
    /// Per-chunk oversized flag. `oversized[i] == true` means chunk `i`
    /// exceeds the threshold because it could not be split safely (typically
    /// a fenced code block larger than the threshold).
    pub oversized: Vec<bool>,
}

impl ChunkPlan {
    /// Slice chunk `idx` out of `content`. Returns `""` if `idx` is out of
    /// range (defensive — callers should validate against `total`).
    pub fn chunk<'a>(&self, content: &'a str, idx: u16) -> &'a str {
        let (start, end) = self.chunk_range(content, idx);
        &content[start..end]
    }

    /// Byte range `[start, end)` for chunk `idx`.
    pub fn chunk_range(&self, content: &str, idx: u16) -> (usize, usize) {
        let idx = idx as usize;
        if idx >= self.total as usize {
            return (content.len(), content.len());
        }
        let start = if idx == 0 {
            0
        } else {
            self.boundaries[idx - 1]
        };
        let end = if idx == self.total as usize - 1 {
            content.len()
        } else {
            self.boundaries[idx]
        };
        (start, end)
    }

    /// Is chunk `idx` flagged oversized? (Over-threshold code block, etc.)
    pub fn is_oversized(&self, idx: u16) -> bool {
        self.oversized.get(idx as usize).copied().unwrap_or(false)
    }

    /// Convenience: iterate chunks as `(idx, &str, oversized_flag)` triples.
    pub fn iter<'a>(&'a self, content: &'a str) -> ChunkPlanIter<'a> {
        ChunkPlanIter {
            plan: self,
            content,
            idx: 0,
        }
    }
}

pub struct ChunkPlanIter<'a> {
    plan: &'a ChunkPlan,
    content: &'a str,
    idx: u16,
}

impl<'a> Iterator for ChunkPlanIter<'a> {
    type Item = (u16, &'a str, bool);
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.plan.total {
            return None;
        }
        let idx = self.idx;
        let chunk = self.plan.chunk(self.content, idx);
        let flag = self.plan.is_oversized(idx);
        self.idx += 1;
        Some((idx, chunk, flag))
    }
}

/// Compute a `ChunkPlan` for `content` at the given `threshold`.
///
/// Deterministic: same `(content, threshold)` → same plan, always. See module
/// docs for the guarantees on the returned plan.
///
/// ## Algorithm
///
/// - Content ≤ threshold → 1 chunk, no boundaries.
/// - Otherwise, walk forward. At each cursor, look for a break point in the
///   window `[cursor, cursor+threshold]`, searching *backwards* from the
///   window end through the preference ladder (see `find_break`). Prefer
///   later breaks within the window to keep chunks as full as practical.
/// - If a chunk would have to land inside a fenced code block and no safe
///   break exists, extend the chunk to the end of the block (may exceed
///   threshold — flagged `oversized`). This is the documented accepted
///   limitation for >28KB code blocks.
pub fn compute_chunks(content: &str, threshold: usize) -> ChunkPlan {
    if content.len() <= threshold {
        return ChunkPlan {
            total: 1,
            boundaries: Vec::new(),
            oversized: vec![false],
        };
    }

    // Pre-compute fence positions so we can reject candidate boundaries inside
    // code blocks without re-scanning from zero each time.
    let fences = find_fence_starts(content);

    let mut boundaries: Vec<usize> = Vec::new();
    let mut oversized: Vec<bool> = Vec::new();
    let mut cursor = 0;

    while content.len() - cursor > threshold {
        let window_end = cursor + threshold;
        match find_break(content, cursor, window_end, &fences) {
            Some(pos) => {
                // Defensive: find_break must produce a break strictly after
                // cursor. If it doesn't (e.g. pathological fence interaction),
                // treat it as a miss and fall through to recovery so we don't
                // emit an empty or backwards chunk.
                if pos <= cursor {
                    let recovery = recover_past_block(content, cursor, window_end, &fences)
                        .unwrap_or_else(|| safe_utf8_fallback(content, content.len()));
                    if recovery <= cursor {
                        oversized.push(true);
                        break;
                    }
                    let chunk_len = recovery - cursor;
                    boundaries.push(recovery);
                    oversized.push(chunk_len > threshold);
                    cursor = recovery;
                    continue;
                }
                let chunk_len = pos - cursor;
                boundaries.push(pos);
                // Mark oversized if the chunk exceeds threshold. Should not
                // happen from the Some branch (find_break caps at window_end)
                // but we flag defensively so an algorithm regression surfaces
                // as a flag rather than a silent oversized chunk.
                oversized.push(chunk_len > threshold);
                cursor = pos;
            }
            None => {
                // No safe break inside the window — we are stuck inside a code
                // block or on a huge line. Extend forward to the end of the
                // blocking code block (if any), then to the nearest safe
                // break after that, to avoid producing an infinite loop and
                // to keep the oversized chunk as tight as possible.
                let recovery = recover_past_block(content, cursor, window_end, &fences)
                    .unwrap_or_else(|| safe_utf8_fallback(content, content.len()));
                if recovery <= cursor {
                    // Absolute pathological case — emit whole remainder.
                    oversized.push(true);
                    break;
                }
                let chunk_len = recovery - cursor;
                boundaries.push(recovery);
                oversized.push(chunk_len > threshold);
                cursor = recovery;
            }
        }
    }

    // Final chunk covers [cursor, content.len()) and, by construction, is
    // ≤ threshold in length (unless we broke out of the loop above with
    // `oversized.push(true); break;` — in which case the final push is the
    // oversized flag for the tail).
    if oversized.len() == boundaries.len() {
        let tail_start = boundaries.last().copied().unwrap_or(0);
        let tail_len = content.len() - tail_start;
        oversized.push(tail_len > threshold);
    }

    // `total` is u16 — plenty of headroom at the 28KB default (65535 chunks
    // = ~1.8GB of bloom content) and covers the low-threshold-override
    // regression Diffi flagged (15KB at threshold=50 needs ~300 chunks).
    // `try_from` still saturates at u16::MAX for the truly pathological
    // case; if that ever happens we emit a single mega-tail rather than
    // corrupting the plan shape, and the oversized-flag sweep below will
    // mark it appropriately.
    let total_u16 = u16::try_from(boundaries.len() + 1).unwrap_or(u16::MAX);

    // Final safety sweep: recompute the oversized flag for every chunk from
    // its actual byte size. The in-loop pushes should already be correct,
    // but a dedicated pass is cheap insurance against an edge-case regression
    // sneaking a ≤-threshold flag through the seams. The chunker is
    // load-bearing (Risk 2) — defence-in-depth is warranted.
    let mut recomputed_oversized = Vec::with_capacity(total_u16 as usize);
    for idx in 0..total_u16 {
        let i = idx as usize;
        let start = if i == 0 { 0 } else { boundaries[i - 1] };
        let end = if i == total_u16 as usize - 1 {
            content.len()
        } else {
            boundaries[i]
        };
        let declared = oversized.get(i).copied().unwrap_or(false);
        let actual = end - start > threshold;
        recomputed_oversized.push(declared || actual);
    }

    ChunkPlan {
        total: total_u16,
        boundaries,
        oversized: recomputed_oversized,
    }
}

/// Scan forward from `start`, returning byte offsets where each fenced code
/// block "event" occurs (fence open or fence close — same marker toggles).
/// We only care about the `\n```` (start-of-line fence) form for block-level
/// detection; inline backticks in normal prose don't matter.
fn find_fence_starts(content: &str) -> Vec<usize> {
    let bytes = content.as_bytes();
    let mut fences = Vec::new();
    let mut i = 0;
    // A fence at offset 0 is valid (file starts with ```).
    while i < bytes.len() {
        let at_line_start = i == 0 || bytes[i - 1] == b'\n';
        if at_line_start && i + 2 < bytes.len() && &bytes[i..i + 3] == b"```" {
            fences.push(i);
            // Advance past this fence line to avoid double-matching.
            match bytes[i..].iter().position(|&b| b == b'\n') {
                Some(nl) => i += nl + 1,
                None => break,
            }
        } else {
            i += 1;
        }
    }
    fences
}

/// Is byte offset `pos` inside a fenced code block (strictly between an
/// opening and closing fence)?
fn is_inside_fence(fences: &[usize], pos: usize) -> bool {
    // Count fence events at offsets < pos. Odd → inside, even → outside.
    let count = fences.iter().take_while(|&&f| f < pos).count();
    count % 2 == 1
}

/// Find the best break offset in `content[start..window_end]` according to the
/// preference ladder. Searches *backwards* from `window_end` for each tier to
/// prefer later breaks within the window (keeps chunks full).
///
/// Ladder (stop at first match that isn't inside a code fence):
///
/// 1. `\n---\n` — horizontal rule. Cleanest semantic break.
/// 2. `\n## `  — H2 header boundary.
/// 3. `\n### ` — H3 header boundary.
/// 4. `\n\n`   — paragraph break.
/// 5. `\n`     — line break (last-resort semantic-ish).
/// 6. Deterministic UTF-8-safe byte fallback at `window_end` rounded down.
///
/// Returns `None` only if the entire window is trapped inside a code fence
/// AND the UTF-8 fallback would land inside the fence too. Caller must
/// recover via `recover_past_block`.
fn find_break(content: &str, start: usize, window_end: usize, fences: &[usize]) -> Option<usize> {
    let window_end = window_end.min(content.len());
    if window_end <= start {
        return None;
    }

    // Patterns in preference order. For each, the break offset is the position
    // of the pattern (the newline), so the break happens *before* the marker.
    // That keeps the separator (e.g. `## Heading`) at the top of the next chunk.
    let ladder: &[&[u8]] = &[b"\n---\n", b"\n## ", b"\n### ", b"\n\n", b"\n"];

    for pat in ladder {
        if let Some(pos) = rfind_in_range(content, start, window_end, pat) {
            // Break position is at the `\n` itself. We split so the chunk
            // before the break ends at `\n` (inclusive) and the next chunk
            // begins at the character after the `\n`.
            let split = pos + 1; // advance past the leading '\n'
            if split <= start || split >= window_end {
                continue;
            }
            if !is_inside_fence(fences, split) && content.is_char_boundary(split) {
                return Some(split);
            }
        }
    }

    // Deterministic UTF-8-safe fallback at window_end.
    let fallback = safe_utf8_fallback(content, window_end);
    if fallback > start && !is_inside_fence(fences, fallback) {
        return Some(fallback);
    }

    None
}

/// Find the latest occurrence of `needle` in `content[start..end]`. Returns
/// the absolute byte offset of the match, or `None`. Used to prefer later
/// breaks within a window.
fn rfind_in_range(content: &str, start: usize, end: usize, needle: &[u8]) -> Option<usize> {
    let end = end.min(content.len());
    if end <= start || needle.is_empty() || end - start < needle.len() {
        return None;
    }
    let haystack = &content.as_bytes()[start..end];
    // Manual rfind scan — no_std-friendly, no regex allocations.
    let n = needle.len();
    let mut i = haystack.len().saturating_sub(n);
    loop {
        if &haystack[i..i + n] == needle {
            return Some(start + i);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

/// Round `pos` down to the nearest UTF-8 char boundary. Panic-free.
fn safe_utf8_fallback(content: &str, pos: usize) -> usize {
    let mut p = pos.min(content.len());
    while p > 0 && !content.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// When `find_break` returns None because we're stuck inside a code fence,
/// extend past the block's closing fence (if we can find one) and then try
/// to find a clean break in the next window. Returns the recovered break
/// offset, or `None` to indicate the caller should give up and emit the
/// remainder as one oversized chunk.
fn recover_past_block(
    content: &str,
    start: usize,
    window_end: usize,
    fences: &[usize],
) -> Option<usize> {
    // Find the first fence offset > window_end. If we're inside a fence at
    // window_end, the next fence is the closing fence.
    let next_fence = *fences.iter().find(|&&f| f > window_end)?;
    let bytes = content.as_bytes();
    // Advance to the end of the fence line so the split lands after the
    // closing ``` line (keeps the whole block in the oversized chunk).
    let after_fence_line = match bytes[next_fence..].iter().position(|&b| b == b'\n') {
        Some(nl) => next_fence + nl + 1,
        None => content.len(),
    };
    let safe = safe_utf8_fallback(content, after_fence_line);
    if safe > start { Some(safe) } else { None }
}

// ============================================================================
// extract_salient_phrase — four-tier never-empty cascade
// ============================================================================

/// Maximum length (in chars, not bytes) for an auto-derived phrase. Keeps the
/// "type the opening line of the next chunk" move actually typable.
const PHRASE_MAX_CHARS: usize = 120;
const SENTENCE_MAX_CHARS: usize = 100;
const LINE_FALLBACK_MAX_CHARS: usize = 80;
const SYNTHETIC_PREFIX_CHARS: usize = 40;

/// Extract a salient phrase from a chunk of bloom content. Never returns
/// empty — even for whitespace-only or empty input, the synthetic fallback
/// produces a stable, deterministic phrase.
///
/// Preference ladder:
///
/// 1. First markdown heading in the chunk (`#`, `##`, `###`, etc.). Since
///    chunk boundaries prefer to split at heading positions, chunks past
///    index 0 frequently begin with a heading.
/// 2. First non-empty sentence (split on `. ` or `\n\n`).
/// 3. First non-empty line, truncated to `LINE_FALLBACK_MAX_CHARS`.
/// 4. Synthetic: `"Part {chunk_idx+1}/{total} — <prefix>"`. Deterministic
///    even for empty input (`<prefix>` collapses to empty string).
///
/// `chunk_idx` and `total` are only used for the synthetic tier. Passing 0/1
/// is fine for test fixtures.
pub fn extract_salient_phrase(content: &str, chunk_idx: u16, total: u16) -> String {
    if let Some(heading) = first_heading(content) {
        return cap_chars(heading.trim(), PHRASE_MAX_CHARS);
    }

    if let Some(sentence) = first_sentence(content) {
        let s = sentence.trim();
        if !s.is_empty() {
            return cap_chars(s, SENTENCE_MAX_CHARS);
        }
    }

    if let Some(line) = first_non_empty_line(content) {
        return cap_chars(line.trim(), LINE_FALLBACK_MAX_CHARS);
    }

    synthetic_phrase(content, chunk_idx, total)
}

fn first_heading(content: &str) -> Option<String> {
    // Use pulldown-cmark to walk the CommonMark AST instead of naive
    // line-prefix scanning. This correctly handles all fence variants:
    // quadruple-backtick fences, tilde fences, indented code blocks, etc.
    // Fixes mx#215 — the old `starts_with("```")` toggle broke for
    // quadruple-backtick fences where inner triple backticks flipped the
    // fence state incorrectly.
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let parser = Parser::new(content);
    let mut in_heading = false;
    let mut heading_text = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                in_heading = true;
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let trimmed = heading_text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
                in_heading = false;
            }
            Event::Text(ref text) | Event::Code(ref text) if in_heading => {
                heading_text.push_str(text);
            }
            _ => {}
        }
    }
    None
}

fn first_sentence(content: &str) -> Option<String> {
    // Only fires when there's a real sentence terminator. Without one we fall
    // through to the line-fallback tier rather than treating an entire blob
    // as a "sentence" (which caps at 100 chars instead of the tighter 80).
    let para_end = content.find("\n\n");
    let sentence_end = content.find(". ").map(|i| i + 1); // keep the period
    let end = match (para_end, sentence_end) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let end = end?;
    let trimmed = content[..end].trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn first_non_empty_line(content: &str) -> Option<String> {
    let mut fence_marker: Option<(char, usize)> = None; // (char, run_length) of opening fence
    for line in content.lines() {
        let trimmed_start = line.trim_start();
        if let Some(marker) = detect_fence_marker(trimmed_start) {
            match fence_marker {
                None => {
                    // Opening a new fence.
                    fence_marker = Some(marker);
                }
                Some((open_ch, open_len)) => {
                    // Only close if same char and at least as many repetitions.
                    if marker.0 == open_ch && marker.1 >= open_len {
                        fence_marker = None;
                    }
                    // Otherwise it's content inside the fence — skip.
                }
            }
            continue;
        }
        if fence_marker.is_some() {
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Detect whether `trimmed_line` (already left-trimmed) is a CommonMark
/// fenced code block marker. Returns `Some((char, run_length))` for the
/// fence character (`` ` `` or `~`) and the number of consecutive
/// occurrences, or `None` if this is not a fence marker.
///
/// CommonMark rules:
/// - At least 3 consecutive backticks or tildes at the start of a line.
/// - The closing fence must use the same character and be at least as long
///   as the opening fence.
/// - Backtick fences may have an info string after the run; tilde fences
///   may too. The closing fence must not contain content after the run
///   (except whitespace), but we don't enforce that here — we're just
///   detecting whether a line *starts* a fence. Callers track open/close
///   state via the `(char, run_length)` tuple.
fn detect_fence_marker(trimmed_line: &str) -> Option<(char, usize)> {
    let first = trimmed_line.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let run_len = trimmed_line.chars().take_while(|&c| c == first).count();
    if run_len >= 3 {
        Some((first, run_len))
    } else {
        None
    }
}

fn synthetic_phrase(content: &str, chunk_idx: u16, total: u16) -> String {
    let prefix: String = content
        .chars()
        .take_while(|c| !matches!(c, '\n' | '\r'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let prefix_capped = cap_chars(prefix.trim(), SYNTHETIC_PREFIX_CHARS);
    let display_idx = chunk_idx.saturating_add(1);
    let display_total = total.max(display_idx);
    if prefix_capped.is_empty() {
        format!("Part {}/{}", display_idx, display_total)
    } else {
        format!("Part {}/{} — {}", display_idx, display_total, prefix_capped)
    }
}

/// Truncate `s` to at most `max` chars (not bytes). Appends `…` if truncated.
/// Truncates at the last word boundary under the cap when possible to avoid
/// mid-word cuts.
fn cap_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    // Collect first `max` chars, then back off to the last whitespace so we
    // don't cut mid-word.
    let head: String = s.chars().take(max).collect();
    let cut = match head.rfind(char::is_whitespace) {
        Some(i) if i >= max / 2 => &head[..i],
        _ => &head[..],
    };
    format!("{}…", cut.trim_end())
}

// ============================================================================
// compare_phrase — exact for authored, tolerant for derived
// ============================================================================

/// Which comparison mode to use. Authored phrases are short, human-curated
/// distillations and stay exact-match (modulo existing fuzzy_match). Derived
/// phrases are longer content samples and get softened comparisons (§5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseMode {
    Authored,
    Derived,
}

/// Comparison outcome. `Exact` and `Tolerant` both mean "accept"; the caller
/// may log which path matched. `Mismatch` means reject (possibly with hints).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseMatch {
    /// Inputs match exactly (after trimming whitespace).
    Exact,
    /// Inputs match only after mode-specific normalization (Derived mode only).
    Tolerant,
    /// No match.
    Mismatch,
}

/// Compare a user-typed `input` against a `target` phrase under the given mode.
///
/// - `Authored`: case-sensitive, whitespace-trimmed exact compare.
/// - `Derived`: additionally lowercase, strip trailing sentence punctuation,
///   collapse internal whitespace runs, normalize smart quotes to straight
///   quotes. These softenings apply to both sides.
///
/// Note: this does NOT replace the existing `engage::fuzzy_match` Levenshtein/
/// word-overlap path. It's the first-pass decision. Callers should still fall
/// through to `fuzzy_match` on `Mismatch` so Partial/Close tiers keep working.
pub fn compare_phrase(input: &str, target: &str, mode: PhraseMode) -> PhraseMatch {
    let i = input.trim();
    let t = target.trim();
    if i == t {
        return PhraseMatch::Exact;
    }
    match mode {
        PhraseMode::Authored => PhraseMatch::Mismatch,
        PhraseMode::Derived => {
            if normalize_derived(i) == normalize_derived(t) {
                PhraseMatch::Tolerant
            } else {
                PhraseMatch::Mismatch
            }
        }
    }
}

/// The per-mode normalizer applied to derived phrases. Public for testing and
/// for any future consumer that wants to preview the normalization.
pub fn normalize_derived(s: &str) -> String {
    // Lowercase, smart-quote → straight-quote, collapse whitespace runs,
    // strip trailing sentence punctuation.
    let lowered = s.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut prev_space = false;
    for ch in lowered.chars() {
        let c = match ch {
            '\u{2018}' | '\u{2019}' | '\u{2032}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{2033}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            _ => ch,
        };
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        prev_space = false;
        out.push(c);
    }
    // Strip trailing sentence-terminators and ellipsis.
    while let Some(last) = out.chars().last() {
        if matches!(last, '.' | '!' | '?' | '…' | ',' | ';' | ':') {
            out.pop();
        } else {
            break;
        }
    }
    out.trim().to_string()
}

// ============================================================================
// extract_auto_phrase — four-tier cascade for phraseless blooms (mx#218)
// ============================================================================

/// Extract a wake phrase from content for a chunk that has neither authored
/// nor derived phrases. This is the mx#218 auto-phrase: ensures every chunk
/// in every bloom has a phrase so the 3-attempt + reveal engagement flow
/// applies universally — no bloom can be `--skip`'d without engagement.
///
/// Four-tier cascade:
///
/// 1. **First markdown heading** (outside fenced code blocks).
/// 2. **Content-hash-seeded sentence selection** — deterministic but varies
///    across blooms (same content = same phrase, different content = different
///    sentence selected).
/// 3. **First non-empty line** truncated to ~80 chars.
/// 4. **Title fallback** — bloom title as the phrase (guaranteed non-empty).
///
/// The function is total: non-empty `title` → non-empty output, always.
pub fn extract_auto_phrase(content: &str, title: &str) -> String {
    // Tier 1: first markdown heading outside fenced code blocks.
    if let Some(heading) = first_heading(content) {
        return cap_chars(heading.trim(), PHRASE_MAX_CHARS);
    }

    // Tier 2: content-hash-seeded sentence selection.
    let sentences = extract_sentences(content);
    if !sentences.is_empty() {
        let refs: Vec<&str> = sentences.iter().map(|s| s.as_str()).collect();
        let idx = select_sentence_index(&refs, content);
        let s = sentences[idx].trim();
        if !s.is_empty() {
            return cap_chars(s, SENTENCE_MAX_CHARS);
        }
    }

    // Tier 3: first non-empty line truncated.
    if let Some(line) = first_non_empty_line(content) {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return cap_chars(trimmed, LINE_FALLBACK_MAX_CHARS);
        }
    }

    // Tier 4: title fallback (guaranteed non-empty by caller contract).
    cap_chars(title.trim(), PHRASE_MAX_CHARS)
}

/// Split content into sentences. A sentence boundary is:
/// - `. ` followed by an uppercase letter or end-of-content
/// - `\n\n` (paragraph break)
///
/// Returns non-empty trimmed sentences. Skips content inside fenced code
/// blocks to avoid picking code comments as sentences.
fn extract_sentences(content: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut fence_marker: Option<(char, usize)> = None;

    for line in content.lines() {
        let trimmed_start = line.trim_start();
        if let Some(marker) = detect_fence_marker(trimmed_start) {
            match fence_marker {
                None => {
                    fence_marker = Some(marker);
                }
                Some((open_ch, open_len)) => {
                    if marker.0 == open_ch && marker.1 >= open_len {
                        fence_marker = None;
                    }
                }
            }
            continue;
        }
        if fence_marker.is_some() {
            continue;
        }

        if line.trim().is_empty() {
            // Paragraph break — flush current sentence.
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
            continue;
        }

        // Strip markdown list-item prefixes so warmth-brick lines like
        // "- kautau noticed the pattern" don't keep the `- ` artifact.
        let mut cleaned = line.trim();
        if let Some(rest) = cleaned.strip_prefix("- ") {
            cleaned = rest;
        } else if let Some(rest) = cleaned.strip_prefix("* ") {
            cleaned = rest;
        }

        // Append line to current accumulator, separated by space.
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(cleaned);

        // Check for `. ` sentence boundaries within the accumulated text.
        // Split greedily on `. ` — each fragment before the last is a sentence.
        while let Some(pos) = current.find(". ") {
            let sentence = current[..=pos].trim().to_string(); // include the period
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            current = current[pos + 2..].to_string();
        }
    }

    // Flush remaining.
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    sentences
}

/// Deterministic sentence index selection based on content hash. Same
/// content always selects the same sentence, but different blooms select
/// different sentences — prevents memorization of "always first sentence."
fn select_sentence_index(sentences: &[&str], content: &str) -> usize {
    let seed: u64 = content
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    seed as usize % sentences.len()
}

// ============================================================================
// Tests — unit, property, cascade, tolerance
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- compute_chunks unit tests --------------------------------------------

    #[test]
    fn below_threshold_is_one_chunk() {
        let plan = compute_chunks("tiny content", 28_000);
        assert_eq!(plan.total, 1);
        assert!(plan.boundaries.is_empty());
        assert_eq!(plan.chunk("tiny content", 0), "tiny content");
        assert!(!plan.is_oversized(0));
    }

    #[test]
    fn exact_threshold_is_one_chunk() {
        let content = "a".repeat(100);
        let plan = compute_chunks(&content, 100);
        assert_eq!(plan.total, 1);
    }

    #[test]
    fn just_over_threshold_chunks_to_size_bound() {
        // No newlines → deterministic UTF-8 fallback at each window. Every
        // chunk except possibly the last is ~threshold bytes. Reconstitution
        // must hold, and no chunk exceeds the threshold.
        let content = "a".repeat(201);
        let plan = compute_chunks(&content, 100);
        assert!(plan.total >= 2);
        let joined: String = plan
            .iter(&content)
            .map(|(_, s, _)| s)
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined, content);
        for (_, chunk, oversized) in plan.iter(&content) {
            if !oversized {
                assert!(chunk.len() <= 100, "chunk {} over threshold", chunk.len());
            }
        }
    }

    #[test]
    fn horizontal_rule_preferred_over_paragraph() {
        // HR at byte 50, paragraph break at byte 70, threshold 100.
        // Backwards search from window_end=100 hits the HR first? No —
        // rfind finds the *latest* occurrence, so paragraph-at-70 loses
        // to HR-at-50 only if HR is strictly preferred (it is, it's tier 1).
        let mut content = String::new();
        content.push_str(&"x".repeat(50));
        content.push_str("\n---\n");
        content.push_str(&"y".repeat(15));
        content.push_str("\n\n");
        content.push_str(&"z".repeat(80));
        let plan = compute_chunks(&content, 100);
        assert!(plan.total >= 2);
        // Tier 1 (HR) should have been chosen → first chunk ends at the `\n`
        // before `---`, so chunk 0 ends with `\n` at byte 51.
        let chunk0 = plan.chunk(&content, 0);
        assert!(
            chunk0.ends_with('\n'),
            "chunk 0 should end at HR newline: {:?}",
            chunk0
        );
    }

    #[test]
    fn falls_through_to_h2_header() {
        // Only an H2 in the window, no HR.
        let mut content = String::new();
        content.push_str(&"x".repeat(50));
        content.push_str("\n## Section Two\n");
        content.push_str(&"y".repeat(200));
        let plan = compute_chunks(&content, 100);
        assert!(plan.total >= 2);
        let chunk1 = plan.chunk(&content, 1);
        assert!(
            chunk1.starts_with("## Section Two"),
            "chunk 1 starts with H2: {:?}",
            &chunk1[..30.min(chunk1.len())]
        );
    }

    #[test]
    fn falls_through_to_paragraph_break() {
        let mut content = String::new();
        content.push_str(&"x".repeat(50));
        content.push_str("\n\n");
        content.push_str(&"y".repeat(150));
        let plan = compute_chunks(&content, 100);
        assert!(plan.total >= 2);
    }

    #[test]
    fn no_semantic_breaks_uses_utf8_fallback() {
        // One long line with no `\n`.
        let content = "A".repeat(250);
        let plan = compute_chunks(&content, 100);
        assert!(plan.total >= 2);
        // Reconstitution must still hold.
        let joined: String = plan
            .iter(&content)
            .map(|(_, s, _)| s)
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined, content);
    }

    #[test]
    fn utf8_boundary_safety_emoji() {
        // Mostly ASCII, with an emoji that *would* straddle byte 100 if we cut naively.
        let prefix = "a".repeat(98);
        let mut content = prefix.clone();
        content.push('\u{1F41F}'); // 4-byte emoji at bytes 98..102
        content.push_str(&"b".repeat(200));
        let plan = compute_chunks(&content, 100);
        // Any boundary must be on a char boundary.
        for &b in &plan.boundaries {
            assert!(
                content.is_char_boundary(b),
                "boundary {} not on char boundary",
                b
            );
        }
        // Reconstitution.
        let joined: String = plan
            .iter(&content)
            .map(|(_, s, _)| s)
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined, content);
    }

    #[test]
    fn code_block_not_split_mid_block() {
        // Code block spanning well past a natural break point.
        let mut content = String::new();
        content.push_str(&"x".repeat(20));
        content.push_str("\n```rust\n");
        content.push_str(&"fn a() {}\n".repeat(20)); // ~200 bytes of code
        content.push_str("```\n");
        content.push_str(&"y".repeat(300));
        let threshold = 120;
        let plan = compute_chunks(&content, threshold);
        // No boundary should land inside the code block.
        let fence_open = content.find("```").unwrap();
        let fence_close = content.rfind("```").unwrap();
        for &b in &plan.boundaries {
            assert!(
                b <= fence_open || b > fence_close,
                "boundary {} landed inside code block [{}, {}]",
                b,
                fence_open,
                fence_close
            );
        }
    }

    #[test]
    fn oversized_code_block_flagged() {
        // A single code block larger than the threshold. We expect the chunk
        // containing it to be flagged oversized.
        let mut content = String::new();
        content.push_str("intro\n\n");
        content.push_str("```\n");
        content.push_str(&"A".repeat(500));
        content.push_str("\n```\n");
        content.push_str(&"tail".repeat(50));
        let plan = compute_chunks(&content, 100);
        assert!(
            plan.oversized.iter().any(|&f| f),
            "expected at least one oversized chunk, got {:?}",
            plan.oversized
        );
    }

    #[test]
    fn chunk_count_beyond_u8_is_not_truncated() {
        // Regression for Diffi's issue #1 on mx#212. Before widening
        // `ChunkPlan.total` to u16, this configuration produced total=255
        // (u8 saturated) while boundaries.len() was far higher — plan.total
        // lied about the chunk count, breaking progress UX and last-chunk
        // indexing downstream.
        //
        // Force ~600 chunks: 15KB content, threshold=25. Well above 255.
        let threshold = 25;
        let content = "a".repeat(15_000);
        let plan = compute_chunks(&content, threshold);

        // The count must be honest: total == boundaries.len() + 1.
        assert_eq!(
            plan.total as usize,
            plan.boundaries.len() + 1,
            "total {} does not match boundaries.len() + 1 = {}",
            plan.total,
            plan.boundaries.len() + 1,
        );
        // And must exceed the old u8 saturation point, proving the fix.
        assert!(
            plan.total > 255,
            "expected >255 chunks to exercise the u8-saturation regression, got {}",
            plan.total
        );

        // Reconstitution holds across the extended range.
        let joined: String = plan
            .iter(&content)
            .map(|(_, s, _)| s)
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined, content);

        // The last chunk is addressable — pre-fix, plan.chunk(content, 254)
        // would return the tail "chunk 254" but thousands of real bytes
        // beyond that point would be silently conflated into it.
        let last_idx = plan.total - 1;
        let last = plan.chunk(&content, last_idx);
        assert!(last.len() <= threshold + 8, "last chunk over threshold");
    }

    #[test]
    fn env_var_overrides_threshold() {
        // Save + restore env var so we don't pollute other tests.
        let prev = env::var(CHUNK_THRESHOLD_ENV).ok();

        unsafe {
            env::set_var(CHUNK_THRESHOLD_ENV, "50");
        }
        assert_eq!(chunk_threshold(), 50);

        unsafe {
            env::set_var(CHUNK_THRESHOLD_ENV, "not-a-number");
        }
        assert_eq!(chunk_threshold(), DEFAULT_CHUNK_THRESHOLD);

        unsafe {
            env::set_var(CHUNK_THRESHOLD_ENV, "0");
        }
        // 0 is rejected (filter n > 0) and we fall back to the default.
        assert_eq!(chunk_threshold(), DEFAULT_CHUNK_THRESHOLD);

        unsafe {
            match prev {
                Some(v) => env::set_var(CHUNK_THRESHOLD_ENV, v),
                None => env::remove_var(CHUNK_THRESHOLD_ENV),
            }
        }
    }

    // --- extract_salient_phrase unit tests ------------------------------------

    #[test]
    fn phrase_from_heading_preferred() {
        let content = "\n## Token semantics\n\nThe token signs (session_id, step).";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "Token semantics");
    }

    #[test]
    fn phrase_heading_strips_all_hash_levels() {
        let content = "#### Deep heading\n\nbody";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "Deep heading");
    }

    #[test]
    fn phrase_from_first_sentence_when_no_heading() {
        let content = "The wake ritual walks a cascade. It uses chunks now.";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "The wake ritual walks a cascade.");
    }

    #[test]
    fn phrase_from_first_line_truncated() {
        // Single giant line, no sentence boundary, no heading. Falls to line
        // fallback which caps at LINE_FALLBACK_MAX_CHARS.
        let content = "word ".repeat(50); // ~250 chars, no period
        let p = extract_salient_phrase(&content, 0, 1);
        assert!(
            p.chars().count() <= LINE_FALLBACK_MAX_CHARS + 1,
            "got {} chars",
            p.chars().count()
        );
        assert!(!p.is_empty());
    }

    #[test]
    fn phrase_synthetic_for_empty_input() {
        let p = extract_salient_phrase("", 0, 3);
        assert_eq!(p, "Part 1/3");
    }

    #[test]
    fn phrase_synthetic_for_whitespace_only() {
        let p = extract_salient_phrase("   \n\n\n   ", 2, 5);
        assert_eq!(p, "Part 3/5");
    }

    #[test]
    fn phrase_never_empty() {
        // Sweep a range of pathological inputs — the total-function guarantee.
        let cases = ["", " ", "\n", "\n\n", "\t\t", "a", ".", "\u{200B}"];
        for c in cases {
            let p = extract_salient_phrase(c, 0, 1);
            assert!(!p.is_empty(), "empty phrase for input {:?}", c);
        }
    }

    #[test]
    fn phrase_abbreviation_splits_at_period_space_known_limitation() {
        // Pinned regression test for the documented (and accepted) limitation
        // that `first_sentence` splits naively on `". "` and therefore cuts
        // abbreviations short. This test exists so a future change that
        // introduces smarter abbrev handling doesn't silently shift behavior
        // without updating the test. If you're reading this because you just
        // broke it: consider whether you meant to, and if yes, update the
        // expected value.
        //
        // Known shortened forms that land here today: Dr. Mr. Mrs. Ms. St.
        // vs. etc. i.e. e.g. — a stop-list pass could improve this. Left as
        // a follow-up (see mx#212 review reply for scope rationale).
        let content = "See Dr. Smith for details. He prescribes two aspirin.";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "See Dr.");
    }

    #[test]
    fn phrase_heading_skips_inside_fenced_code_block() {
        // Regression for Diffi's issue #2 on mx#212. The prose paragraph has
        // no real heading; the only `## ...` in the chunk is inside a fenced
        // block and must NOT be picked as the salient phrase.
        let content = "Intro paragraph without a heading. More prose here.\n\n\
             ```markdown\n\
             ## fake heading in code\n\
             more code lines\n\
             ```\n\
             trailing prose line.";
        let p = extract_salient_phrase(content, 0, 1);
        assert!(
            !p.contains("fake heading in code"),
            "heading extractor descended into fenced block: {:?}",
            p
        );
        // With no real heading, we expect the first sentence of the prose.
        assert!(
            p.starts_with("Intro paragraph"),
            "expected first-sentence fallback, got {:?}",
            p
        );
    }

    #[test]
    fn phrase_heading_after_fenced_block_is_picked() {
        // A real heading that appears *after* a code block should still be
        // picked. Verifies the fence toggle closes properly.
        let content = "Intro prose.\n\n\
             ```rust\n\
             // ## not a heading\n\
             fn x() {}\n\
             ```\n\n\
             ## Real Heading\n\n\
             body text.";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "Real Heading");
    }

    #[test]
    fn phrase_heading_between_two_fenced_blocks_is_picked() {
        // Fence open → fence close → real heading → fence open → fence close.
        // Must pick the real heading sandwiched between the two blocks.
        let content = "\
            ```\n\
            ## fake one\n\
            ```\n\n\
            ## Real Heading\n\n\
            body\n\n\
            ```\n\
            ## fake two\n\
            ```\n";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "Real Heading");
    }

    #[test]
    fn phrase_pure_fenced_chunk_has_no_extractable_heading() {
        // Chunk is nothing but a fenced block containing a fake heading.
        // Heading tier must skip; we fall through to sentence/line/synthetic.
        let content = "\
            ```markdown\n\
            ## fake heading inside code\n\
            more code\n\
            ```\n";
        let p = extract_salient_phrase(content, 0, 1);
        assert!(
            !p.contains("fake heading inside code"),
            "fence-only chunk returned fake heading: {:?}",
            p
        );
        assert!(!p.is_empty());
    }

    // --- mx#215 CommonMark fence edge-case tests --------------------------------

    #[test]
    fn heading_quadruple_backtick_fence_ignores_inner_triple() {
        // The bug: quadruple-backtick fences contain inner triple backticks
        // that the naive `starts_with("```")` toggle treated as fence
        // close/open, causing a fake heading inside the block to be picked.
        let content = "\
````markdown
Here is how you write a fenced block:

```
## This heading is inside the inner fence
```

And that's it.
````

## Real Heading After Quad Fence

Body text.";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(
            p, "Real Heading After Quad Fence",
            "should skip heading inside quadruple-backtick fence, got {:?}",
            p
        );
    }

    #[test]
    fn heading_tilde_fence_ignored() {
        // Tilde fences are valid CommonMark but were never handled by the
        // old `starts_with("```")` check.
        let content = "\
~~~
## Heading inside tilde fence
~~~

## Real Tilde Heading

Body.";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "Real Tilde Heading");
    }

    #[test]
    fn heading_inside_code_block_at_all_levels_ignored() {
        let content = "\
```
# H1 fake
## H2 fake
### H3 fake
#### H4 fake
```

### Real H3 Heading

Content.";
        let p = extract_salient_phrase(content, 0, 1);
        assert_eq!(p, "Real H3 Heading");
    }

    #[test]
    fn no_heading_returns_none_from_first_heading() {
        let content = "Just some text without any heading markers.";
        assert!(first_heading(content).is_none());
    }

    #[test]
    fn heading_levels_h1_through_h6() {
        assert_eq!(first_heading("# H1").unwrap(), "H1");
        assert_eq!(first_heading("## H2").unwrap(), "H2");
        assert_eq!(first_heading("### H3").unwrap(), "H3");
        assert_eq!(first_heading("#### H4").unwrap(), "H4");
        assert_eq!(first_heading("##### H5").unwrap(), "H5");
        assert_eq!(first_heading("###### H6").unwrap(), "H6");
    }

    #[test]
    fn first_non_empty_line_quadruple_fence() {
        // first_non_empty_line should also handle quad fences correctly.
        let content = "\
````
## fake heading
```
inner triple
```
````

Actual first line.";
        let line = first_non_empty_line(content);
        assert_eq!(
            line.as_deref(),
            Some("Actual first line."),
            "first_non_empty_line should handle quad fences, got {:?}",
            line
        );
    }

    #[test]
    fn extract_sentences_quadruple_fence() {
        // extract_sentences should also handle quad fences correctly.
        let content = "\
````
Fake sentence inside quad fence. Another fake.
```
More fake inside inner triple.
```
````

Real sentence outside. Another real one.";
        let sentences = extract_sentences(content);
        for s in &sentences {
            assert!(
                !s.contains("Fake sentence"),
                "extract_sentences should skip quad-fenced content, got {:?}",
                s
            );
        }
        assert!(
            sentences.iter().any(|s| s.contains("Real sentence")),
            "should find real sentence outside fence, got {:?}",
            sentences
        );
    }

    #[test]
    fn extract_sentences_tilde_fence() {
        let content = "\
~~~
Fake sentence inside tilde fence.
~~~

Real tilde sentence.";
        let sentences = extract_sentences(content);
        for s in &sentences {
            assert!(
                !s.contains("Fake sentence"),
                "should skip tilde-fenced content, got {:?}",
                s
            );
        }
    }

    // --- compare_phrase unit tests --------------------------------------------

    #[test]
    fn compare_authored_exact_match() {
        let r = compare_phrase(
            "Rust is memory-safe",
            "Rust is memory-safe",
            PhraseMode::Authored,
        );
        assert_eq!(r, PhraseMatch::Exact);
    }

    #[test]
    fn compare_authored_case_mismatch_is_reject() {
        let r = compare_phrase(
            "rust is memory-safe",
            "Rust is memory-safe",
            PhraseMode::Authored,
        );
        assert_eq!(r, PhraseMatch::Mismatch);
    }

    #[test]
    fn compare_derived_case_tolerant() {
        let r = compare_phrase("Token semantics", "token semantics", PhraseMode::Derived);
        assert_eq!(r, PhraseMatch::Tolerant);
    }

    #[test]
    fn compare_derived_trailing_punct_stripped() {
        let r = compare_phrase(
            "The wake ritual walks a cascade",
            "The wake ritual walks a cascade.",
            PhraseMode::Derived,
        );
        assert_eq!(r, PhraseMatch::Tolerant);
    }

    #[test]
    fn compare_derived_whitespace_collapsed() {
        let r = compare_phrase("token   semantics", "token semantics", PhraseMode::Derived);
        assert_eq!(r, PhraseMatch::Tolerant);
    }

    #[test]
    fn compare_derived_smart_quotes_normalized() {
        let r = compare_phrase(
            "it's \u{201C}alive\u{201D}",
            "it\u{2019}s \"alive\"",
            PhraseMode::Derived,
        );
        assert_eq!(r, PhraseMatch::Tolerant);
    }

    #[test]
    fn compare_derived_mismatch_still_mismatches() {
        let r = compare_phrase("totally different", "token semantics", PhraseMode::Derived);
        assert_eq!(r, PhraseMatch::Mismatch);
    }

    // --- property tests -------------------------------------------------------

    // These are Risk 2 in the design — non-negotiable.

    use proptest::prelude::*;

    fn threshold_strategy() -> impl Strategy<Value = usize> {
        // Small thresholds stress boundary logic; large thresholds would be
        // slow without revealing more.
        prop_oneof![
            Just(50usize),
            Just(100usize),
            Just(256usize),
            Just(1024usize),
            Just(4096usize),
        ]
    }

    proptest! {
        // Risk 2 — the load-bearing invariant. If this ever fails we have a
        // silent-content-corruption bug. `join(chunks) == original`.
        #[test]
        fn prop_reconstitution(
            content in "\\PC{0,8192}", // up to 8KB of any printable + control chars
            threshold in threshold_strategy(),
        ) {
            let plan = compute_chunks(&content, threshold);
            let joined: String = plan.iter(&content).map(|(_, s, _)| s).collect::<Vec<_>>().join("");
            prop_assert_eq!(joined, content);
        }

        #[test]
        fn prop_all_boundaries_are_char_boundaries(
            content in "\\PC{0,8192}",
            threshold in threshold_strategy(),
        ) {
            let plan = compute_chunks(&content, threshold);
            for &b in &plan.boundaries {
                prop_assert!(content.is_char_boundary(b), "byte {} is not a char boundary", b);
            }
        }

        #[test]
        fn prop_boundaries_strictly_increasing(
            content in "\\PC{0,8192}",
            threshold in threshold_strategy(),
        ) {
            let plan = compute_chunks(&content, threshold);
            for pair in plan.boundaries.windows(2) {
                prop_assert!(pair[0] < pair[1], "non-monotonic boundaries: {:?}", plan.boundaries);
            }
        }

        #[test]
        fn prop_determinism(
            content in "\\PC{0,4096}",
            threshold in threshold_strategy(),
        ) {
            let a = compute_chunks(&content, threshold);
            let b = compute_chunks(&content, threshold);
            prop_assert_eq!(a, b);
        }

        #[test]
        fn prop_chunk_size_bound(
            content in "\\PC{0,8192}",
            threshold in threshold_strategy(),
        ) {
            // Non-oversized chunks must be ≤ threshold. Oversized chunks are
            // an accepted limitation (over-threshold code blocks, pathological
            // fence content) and bounded only by the content length itself —
            // the contract is "if we couldn't split safely, we flagged it."
            let plan = compute_chunks(&content, threshold);
            for (_, chunk, oversized) in plan.iter(&content) {
                if oversized {
                    prop_assert!(
                        chunk.len() <= content.len(),
                        "oversized chunk larger than input: {} vs {}",
                        chunk.len(), content.len()
                    );
                } else {
                    prop_assert!(
                        chunk.len() <= threshold,
                        "non-oversized chunk {} > threshold {}",
                        chunk.len(), threshold
                    );
                }
            }
        }

        #[test]
        fn prop_phrase_never_empty(content in "\\PC{0,4096}", idx in 0u16..10, total in 1u16..10) {
            let p = extract_salient_phrase(&content, idx, total);
            prop_assert!(!p.is_empty(), "empty phrase for content len {}", content.len());
        }

        #[test]
        fn prop_phrase_deterministic(content in "\\PC{0,4096}") {
            let a = extract_salient_phrase(&content, 0, 1);
            let b = extract_salient_phrase(&content, 0, 1);
            prop_assert_eq!(a, b);
        }

        #[test]
        fn prop_compare_derived_tolerant_to_case_and_trailing_punct(
            word1 in "[a-zA-Z]{2,20}",
            word2 in "[a-zA-Z]{2,20}",
        ) {
            let base = format!("{} {}", word1, word2);
            let variant = format!("{} {}.", base.to_lowercase(), ""); // lowercase + trailing period
            let variant = variant.trim().to_string();
            let r = compare_phrase(&variant, &base, PhraseMode::Derived);
            prop_assert!(matches!(r, PhraseMatch::Exact | PhraseMatch::Tolerant),
                "derived compare rejected trivial variant: {:?} vs {:?}", variant, base);
        }

        #[test]
        fn prop_compare_same_string_is_exact(s in "[\\PC]{1,64}") {
            let trimmed = s.trim().to_string();
            prop_assume!(!trimmed.is_empty());
            prop_assert_eq!(
                compare_phrase(&trimmed, &trimmed, PhraseMode::Authored),
                PhraseMatch::Exact
            );
            prop_assert_eq!(
                compare_phrase(&trimmed, &trimmed, PhraseMode::Derived),
                PhraseMatch::Exact
            );
        }
    }

    // --- extract_auto_phrase unit tests (mx#218) ---------------------------------

    #[test]
    fn auto_phrase_from_heading() {
        let content = "Some intro text.\n\n## The Spark\n\nBody text here.";
        let p = extract_auto_phrase(content, "Fallback Title");
        assert_eq!(p, "The Spark");
    }

    #[test]
    fn auto_phrase_from_sentence() {
        // No heading — should fall through to sentence selection.
        let content = "The warmth accumulator stores relational bricks. Each brick records a moment of connection.";
        let p = extract_auto_phrase(content, "Warmth Accumulator");
        // Must be one of the two sentences, deterministically selected.
        assert!(
            p.contains("warmth accumulator") || p.contains("brick records"),
            "expected a sentence from the content, got {:?}",
            p
        );
        assert!(!p.is_empty());
    }

    #[test]
    fn auto_phrase_from_line() {
        // No headings, no sentence terminators — falls to first non-empty line.
        let content = "- brick one: kautau noticed the pattern\n- brick two: something else";
        let p = extract_auto_phrase(content, "Fallback");
        assert!(
            p.contains("brick one"),
            "expected first line as phrase, got {:?}",
            p
        );
    }

    #[test]
    fn auto_phrase_from_title_fallback() {
        // Empty content — must fall back to title.
        let p = extract_auto_phrase("", "Warmth Accumulator");
        assert_eq!(p, "Warmth Accumulator");
    }

    #[test]
    fn auto_phrase_never_empty() {
        // Non-empty title always produces a non-empty phrase.
        let cases = ["", " ", "\n", "\n\n", "\t\t"];
        for c in cases {
            let p = extract_auto_phrase(c, "Title");
            assert!(!p.is_empty(), "empty auto-phrase for content {:?}", c);
        }
    }

    #[test]
    fn auto_phrase_deterministic() {
        let content = "Some paragraph with multiple sentences. Another one here. And a third.";
        let a = extract_auto_phrase(content, "Title");
        let b = extract_auto_phrase(content, "Title");
        assert_eq!(a, b, "auto-phrase must be deterministic");
    }

    #[test]
    fn auto_phrase_skips_fenced_headings() {
        // Heading inside a code block must not be selected.
        let content = "```markdown\n## Fake Heading\n```\n\nReal first sentence here.";
        let p = extract_auto_phrase(content, "Title");
        assert!(
            !p.contains("Fake Heading"),
            "auto-phrase picked heading inside fenced block: {:?}",
            p
        );
    }

    #[test]
    fn auto_phrase_warmth_bricks() {
        // A list of `- ` prefixed bricks with no heading or sentence boundaries.
        let content = "- kautau noticed the pattern and said so\n- Q remembered the first wake\n- Semvii brought coffee";
        let p = extract_auto_phrase(content, "Warmth Accumulator");
        // Should pick a brick line (first non-empty line tier or sentence tier).
        assert!(
            p.contains("kautau") || p.contains("Q remembered") || p.contains("Semvii"),
            "expected a brick line, got {:?}",
            p
        );
        assert!(!p.is_empty());
    }

    #[test]
    fn auto_phrase_content_hash_varies_across_blooms() {
        // Different content should (usually) select different sentences.
        // Not a hard guarantee (hash collisions exist) but for these two
        // distinct inputs the seed should differ.
        let content_a = "First sentence here. Second sentence here. Third sentence here.";
        let content_b = "Alpha sentence here. Beta sentence here. Gamma sentence here.";
        let p_a = extract_auto_phrase(content_a, "A");
        let p_b = extract_auto_phrase(content_b, "B");
        // They CAN be the same index by coincidence, but the phrases themselves
        // will differ because the content differs.
        assert_ne!(
            p_a, p_b,
            "different content should produce different phrases"
        );
    }

    // --- extract_sentences unit tests ---

    #[test]
    fn extract_sentences_basic() {
        let content = "First sentence. Second sentence. Third.";
        let sentences = extract_sentences(content);
        assert!(
            sentences.len() >= 2,
            "expected >=2 sentences, got {:?}",
            sentences
        );
        assert!(sentences[0].contains("First sentence."));
    }

    #[test]
    fn extract_sentences_paragraph_break() {
        let content = "Paragraph one\n\nParagraph two";
        let sentences = extract_sentences(content);
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0], "Paragraph one");
        assert_eq!(sentences[1], "Paragraph two");
    }

    #[test]
    fn extract_sentences_skips_fenced_blocks() {
        let content = "Real sentence.\n\n```\nFake sentence inside code.\n```\n\nAnother real one.";
        let sentences = extract_sentences(content);
        for s in &sentences {
            assert!(
                !s.contains("Fake sentence"),
                "sentence extractor should skip fenced blocks, got {:?}",
                s
            );
        }
    }

    #[test]
    fn select_sentence_index_deterministic() {
        let sentences = vec!["a", "b", "c", "d", "e"];
        let content = "some content for hashing";
        let idx1 = select_sentence_index(&sentences, content);
        let idx2 = select_sentence_index(&sentences, content);
        assert_eq!(idx1, idx2);
        assert!(idx1 < sentences.len());
    }

    #[test]
    fn select_sentence_index_varies_with_content() {
        let sentences = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let idx1 = select_sentence_index(&sentences, "content alpha");
        let idx2 = select_sentence_index(&sentences, "content beta");
        // With 10 choices and different seeds, these should differ.
        // Not a hard guarantee but very likely with this hash function.
        assert_ne!(
            idx1, idx2,
            "different content should usually select different indices"
        );
    }

    // --- first_non_empty_line fence-skip tests (Diffi review #221 fix 2) ---

    #[test]
    fn first_non_empty_line_skips_fenced_code_block() {
        // Content that starts with a fenced code block — the first non-empty
        // line outside the fence should be returned, not "```rust".
        let content = "```rust\nfn main() {}\n```\n\nActual first line.";
        let line = first_non_empty_line(content);
        assert_eq!(
            line.as_deref(),
            Some("Actual first line."),
            "should skip fenced code block, got {:?}",
            line
        );
    }

    #[test]
    fn first_non_empty_line_all_fenced_returns_none() {
        // If the entire content is inside a code fence, no non-empty line
        // exists outside of it.
        let content = "```\nonly code here\nmore code\n```";
        let line = first_non_empty_line(content);
        assert_eq!(line, None, "all-fenced content should return None");
    }

    #[test]
    fn first_non_empty_line_between_fences() {
        let content = "```\ncode\n```\n\nSandwiched line\n\n```\nmore code\n```";
        let line = first_non_empty_line(content);
        assert_eq!(line.as_deref(), Some("Sandwiched line"));
    }

    // --- extract_sentences list-item stripping tests (Diffi review #221 fix 5) ---

    #[test]
    fn extract_sentences_strips_dash_list_prefix() {
        let content = "- first brick line\n- second brick line";
        let sentences = extract_sentences(content);
        for s in &sentences {
            assert!(
                !s.starts_with("- "),
                "sentence should not keep `- ` prefix: {:?}",
                s
            );
        }
        // The two lines join into one sentence (no paragraph break or `. `).
        assert!(!sentences.is_empty());
        assert!(
            sentences[0].contains("first brick line"),
            "expected content without prefix, got {:?}",
            sentences
        );
    }

    #[test]
    fn extract_sentences_strips_star_list_prefix() {
        let content = "* item alpha\n* item beta";
        let sentences = extract_sentences(content);
        for s in &sentences {
            assert!(
                !s.starts_with("* "),
                "sentence should not keep `* ` prefix: {:?}",
                s
            );
        }
        assert!(!sentences.is_empty());
        assert!(sentences[0].contains("item alpha"));
    }

    #[test]
    fn extract_sentences_list_items_with_paragraph_breaks() {
        let content = "- first item\n\n- second item";
        let sentences = extract_sentences(content);
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0], "first item");
        assert_eq!(sentences[1], "second item");
    }
}
