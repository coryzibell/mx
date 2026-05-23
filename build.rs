use std::env;
use std::fs;
use std::path::Path;

use base_d::{CompressionAlgorithm, compress};

/// Zstd compression level for embedded docs. Level 3 is the zstd default —
/// a good balance of speed and ratio for build-time asset compression.
const ZSTD_COMPRESSION_LEVEL: i32 = 3;

fn main() {
    let src = Path::new("docs/out/llm/mx-docs.md");
    println!("cargo:rerun-if-changed={}", src.display());

    let raw = fs::read(src)
        .expect("docs/out/llm/mx-docs.md must exist at build time — run docs/build.sh first");

    let compressed = compress(&raw, CompressionAlgorithm::Zstd, ZSTD_COMPRESSION_LEVEL)
        .expect("zstd compression failed");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("mx-docs.md.zst");
    fs::write(&dest, &compressed).expect("failed to write compressed docs");
}
