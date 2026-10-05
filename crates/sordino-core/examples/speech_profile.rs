//! Measure the long-term speech profile of a recording with the same analyser and gating the
//! automatic correction uses at runtime (after the noise model).
//!
//! `cargo run --release -p sordino-core --example speech_profile -- speech.f32`  (mono 48 kHz f32le)

use sordino_core::autoeq::{Analyzer, BANDS};
use sordino_core::denoise::{Denoiser, Strength};
use sordino_core::HOP;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: speech_profile file.f32"))?;
    let x: Vec<f32> = std::fs::read(path)?
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let mut den = Denoiser::new(Strength::High)?;
    let mut an = Analyzer::new();
    let mut out = [0.0f32; HOP];
    for hop in x.chunks_exact(HOP) {
        let lsnr = den.process_hop(hop, &mut out)?;
        let rms = (out.iter().map(|v| v * v).sum::<f32>() / HOP as f32).sqrt();
        an.feed(&out, lsnr > 15.0 && rms > 1e-3, lsnr < 0.0);
    }
    println!("speech hops: {}", an.speech_hops());
    let p = an.profile();
    for (f, v) in BANDS.iter().zip(p.iter()) {
        println!("{f:7.0} Hz  {v:+6.1} dB");
    }
    println!(
        "TARGET_PROFILE = {:?}",
        p.map(|v| (v * 10.0).round() / 10.0)
    );
    Ok(())
}
