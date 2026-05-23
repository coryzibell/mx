//! Embedded compressed assets baked into the binary at build time.

const DOCS_COMPRESSED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mx-docs.md.zst"));

/// Decompress and return the full mx documentation as a Markdown string.
///
/// Panics if the embedded blob is corrupt — a corrupt blob means the
/// binary itself is broken, so there's nothing to recover from.
pub fn docs_markdown() -> String {
    let bytes = base_d::decompress(DOCS_COMPRESSED, base_d::CompressionAlgorithm::Zstd)
        .expect("embedded docs blob is corrupt — the binary is broken");
    String::from_utf8(bytes)
        .expect("embedded docs blob is not valid UTF-8 — the binary is broken")
}
