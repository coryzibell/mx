use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

fn main() -> Result<()> {
    let test_string = "Hello, world!";
    println!("=== fastembed reference embedding ===");
    println!("Test string: {:?}", test_string);

    let mut model = TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::BGEBaseENV15).with_show_download_progress(false),
    )?;

    let embeddings = model.embed(vec![test_string], None)?;
    let embedding = &embeddings[0];

    println!("Embedding dimension: {}", embedding.len());
    println!("First 10 values:");
    for (i, v) in embedding.iter().take(10).enumerate() {
        println!("  [{:>3}] {:.8}", i, v);
    }

    // Print all values as a compact JSON-like list for comparison
    let l2_norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("L2 norm: {:.8}", l2_norm);

    // Print in a format that's easy to diff
    println!("\n--- COPY BELOW FOR COMPARISON ---");
    for v in embedding.iter().take(20) {
        println!("{:.8}", v);
    }

    Ok(())
}
