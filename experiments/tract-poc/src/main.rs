use anyhow::{Context, Result};
use std::time::Instant;
use tract_onnx::prelude::*;

const MODEL_PATH: &str = "/home/charlie/.cache/fastembed/models--Xenova--bge-base-en-v1.5/snapshots/4d6cd88e18e51a5e020c2c305726d76ada9c03cf/onnx/model.onnx";
const TOKENIZER_PATH: &str = "/home/charlie/.cache/fastembed/models--Xenova--bge-base-en-v1.5/snapshots/4d6cd88e18e51a5e020c2c305726d76ada9c03cf/tokenizer.json";

fn main() -> Result<()> {
    let test_string = "Hello, world!";
    println!("=== tract-onnx BGE-base-en-v1.5 POC ===");
    println!("Test string: {:?}", test_string);
    println!();

    // --- Step 1: Load tokenizer ---
    println!("[1/4] Loading tokenizer...");
    let t0 = Instant::now();
    let tokenizer = tokenizers::Tokenizer::from_file(TOKENIZER_PATH)
        .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;
    println!("  Tokenizer loaded in {:?}", t0.elapsed());

    // --- Step 2: Tokenize ---
    println!("[2/4] Tokenizing...");
    let encoding = tokenizer
        .encode(test_string, true)
        .map_err(|e| anyhow::anyhow!("Failed to tokenize: {}", e))?;

    let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
    let attention_mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&x| x as i64).collect();
    let token_type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&x| x as i64).collect();
    let seq_len = input_ids.len();

    println!("  Tokens: {:?}", encoding.get_tokens());
    println!("  input_ids: {:?}", input_ids);
    println!("  attention_mask: {:?}", attention_mask);
    println!("  token_type_ids: {:?}", token_type_ids);
    println!("  seq_len: {}", seq_len);
    println!();

    // --- Step 3: Load ONNX model with tract ---
    println!("[3/4] Loading ONNX model with tract...");
    let t1 = Instant::now();

    // First, load the raw model to inspect it
    let raw_model = tract_onnx::onnx()
        .model_for_path(MODEL_PATH)
        .context("Failed to load ONNX model")?;

    println!("  Raw model loaded in {:?}", t1.elapsed());
    println!("  Model inputs:");
    for (i, input) in raw_model.input_outlets()?.iter().enumerate() {
        let fact = raw_model.outlet_fact(*input)?;
        let name = &raw_model.node(input.node).name;
        println!("    [{}] name={:?}, fact={:?}", i, name, fact);
    }
    println!("  Model outputs:");
    for (i, output) in raw_model.output_outlets()?.iter().enumerate() {
        let fact = raw_model.outlet_fact(*output)?;
        let name = &raw_model.node(output.node).name;
        println!("    [{}] name={:?}, fact={:?}", i, name, fact);
    }
    println!();

    // Set fixed input facts for batch=1, seq_len=actual
    println!("  Setting input facts (batch=1, seq_len={})...", seq_len);
    let t2 = Instant::now();

    let mut model = raw_model;
    let batch = 1_usize;

    // BERT models typically have 3 inputs: input_ids, attention_mask, token_type_ids
    // They should all be i64 [batch, seq_len]
    let input_fact = InferenceFact::dt_shape(i64::datum_type(), tvec![batch.to_dim(), seq_len.to_dim()]);

    // Set facts for all 3 inputs
    for i in 0..3 {
        model.set_input_fact(i, input_fact.clone())
            .with_context(|| format!("Failed to set input fact for input {}", i))?;
    }

    let model = model
        .into_optimized()
        .context("Failed to optimize model")?;

    let model = model
        .into_runnable()
        .context("Failed to make model runnable")?;

    println!("  Model optimized and ready in {:?}", t2.elapsed());
    println!();

    // --- Step 4: Run inference ---
    println!("[4/4] Running inference...");
    let t3 = Instant::now();

    // Create input tensors
    let input_ids_tensor = tract_ndarray::Array2::from_shape_vec(
        (batch, seq_len),
        input_ids.clone(),
    )?.into_tensor();

    let attention_mask_tensor = tract_ndarray::Array2::from_shape_vec(
        (batch, seq_len),
        attention_mask.clone(),
    )?.into_tensor();

    let token_type_ids_tensor = tract_ndarray::Array2::from_shape_vec(
        (batch, seq_len),
        token_type_ids.clone(),
    )?.into_tensor();

    let inputs = tvec![
        input_ids_tensor.into(),
        attention_mask_tensor.into(),
        token_type_ids_tensor.into(),
    ];

    let outputs = model.run(inputs).context("Failed to run inference")?;
    let inference_time = t3.elapsed();
    println!("  Inference completed in {:?}", inference_time);

    // --- Step 5: Extract and normalize embedding ---
    println!();
    println!("=== Results ===");
    println!("  Number of output tensors: {}", outputs.len());

    for (i, output) in outputs.iter().enumerate() {
        println!("  Output[{}] shape: {:?}, dtype: {:?}", i, output.shape(), output.datum_type());
    }

    // The model output is [batch, seq_len, 768]
    let output_tensor = outputs[0]
        .to_array_view::<f32>()
        .context("Failed to convert output to f32 array")?;

    println!("  Output tensor shape: {:?}", output_tensor.shape());

    let hidden_size = output_tensor.shape()[2]; // 768
    println!("  Hidden size: {}", hidden_size);

    // === CLS Pooling (what fastembed uses by default for BGE) ===
    // Take the [CLS] token output (position 0)
    let mut cls_pooled = vec![0.0f32; hidden_size];
    for hidden_idx in 0..hidden_size {
        cls_pooled[hidden_idx] = output_tensor[[0, 0, hidden_idx]];
    }

    // L2 normalize CLS
    let cls_l2: f32 = cls_pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
    if cls_l2 > 0.0 {
        for v in cls_pooled.iter_mut() {
            *v /= cls_l2;
        }
    }

    // === Mean Pooling (alternative) ===
    let mut mean_pooled = vec![0.0f32; hidden_size];
    let mut mask_sum = 0.0f32;
    for seq_idx in 0..seq_len {
        let mask_val = attention_mask[seq_idx] as f32;
        mask_sum += mask_val;
        for hidden_idx in 0..hidden_size {
            mean_pooled[hidden_idx] += output_tensor[[0, seq_idx, hidden_idx]] * mask_val;
        }
    }
    if mask_sum > 0.0 {
        for v in mean_pooled.iter_mut() {
            *v /= mask_sum;
        }
    }
    let mean_l2: f32 = mean_pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mean_l2 > 0.0 {
        for v in mean_pooled.iter_mut() {
            *v /= mean_l2;
        }
    }

    // === fastembed reference values for "Hello, world!" ===
    // (produced by fastembed 5.13 with default CLS pooling)
    let fastembed_ref: [f32; 20] = [
        0.00727994, 0.03128255, 0.04226948, -0.00634848, -0.00775309,
        0.04410859, 0.05909877, 0.04548819, -0.02227267, -0.03785515,
        -0.00671289, -0.00336306, -0.08337980, -0.00515877, 0.01829065,
        0.05117791, 0.03137933, 0.01345143, 0.00620514, 0.03658377,
    ];

    println!();
    println!("=== CLS Pooling (fastembed-compatible) ===");
    println!("  First 20 values (tract vs fastembed reference):");
    let mut max_diff_cls = 0.0f32;
    for i in 0..20 {
        let diff = (cls_pooled[i] - fastembed_ref[i]).abs();
        if diff > max_diff_cls {
            max_diff_cls = diff;
        }
        println!(
            "    [{:>3}] tract={:>12.8}  fastembed={:>12.8}  diff={:.2e}",
            i, cls_pooled[i], fastembed_ref[i], diff
        );
    }
    let cls_norm: f32 = cls_pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("  L2 norm: {:.8} (should be ~1.0)", cls_norm);
    println!("  Max diff (first 20): {:.2e}", max_diff_cls);

    // Compute cosine similarity between tract CLS and fastembed ref (first 20 dims only)
    let dot: f32 = cls_pooled.iter().zip(fastembed_ref.iter()).map(|(a, b)| a * b).sum();
    let norm_a: f32 = cls_pooled.iter().take(20).map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = fastembed_ref.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("  Cosine similarity (first 20 dims): {:.8}", dot / (norm_a * norm_b));

    println!();
    println!("=== Mean Pooling (alternative) ===");
    println!("  First 10 values:");
    for (i, v) in mean_pooled.iter().take(10).enumerate() {
        println!("    [{:>3}] {:.8}", i, v);
    }
    let mean_norm: f32 = mean_pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("  L2 norm: {:.8}", mean_norm);

    println!();
    println!("  Embedding dimension: {}", hidden_size);

    println!();
    if max_diff_cls < 1e-4 {
        println!("=== RESULT: PASS -- tract output matches fastembed within 1e-4 tolerance ===");
    } else if max_diff_cls < 1e-2 {
        println!("=== RESULT: CLOSE -- tract output close to fastembed (max diff {:.2e}) ===", max_diff_cls);
    } else {
        println!("=== RESULT: MISMATCH -- tract output differs from fastembed (max diff {:.2e}) ===", max_diff_cls);
    }
    println!();
    println!("=== POC COMPLETE: tract-onnx successfully loaded and ran BGE-base-en-v1.5 ===");

    Ok(())
}
