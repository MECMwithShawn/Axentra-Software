//! Halve the shipped model: `cargo run -p axentra-embed --example to_f16 -- in.safetensors out.safetensors`.
//! The app reads it back as f32; search ranks the same.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tensors = candle_core::safetensors::load(&args[1], &candle_core::Device::Cpu).unwrap();
    let half: std::collections::HashMap<String, candle_core::Tensor> =
        tensors.into_iter().map(|(k, t)| (k, t.to_dtype(candle_core::DType::F16).unwrap())).collect();
    candle_core::safetensors::save(&half, &args[2]).unwrap();
}
