use std::env;
use std::fs;
use std::path::Path;

use base_d::{CompressionAlgorithm, compress};

fn main() {
    let src = Path::new("docs/out/llm/mx-docs.md");
    println!("cargo:rerun-if-changed={}", src.display());

    let raw = fs::read(src).expect("docs/out/llm/mx-docs.md must exist at build time — run docs/build.sh first");

    let compressed = compress(&raw, CompressionAlgorithm::Zstd, 3)
        .expect("zstd compression failed");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("mx-docs.md.zst");
    fs::write(&dest, &compressed).expect("failed to write compressed docs");
}
