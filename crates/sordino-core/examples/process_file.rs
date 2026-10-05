//! Run the Sordino pipeline offline on a file, for analysis and tuning.
//!
//! `process_file in.f32 out.f32 [--noise off|light|medium|high|max] [--studio off|natural|clear|warm]
//!                          [--echo ref.f32] [--thresh min,erb,df] [--autoeq on|off]
//!                          [--pause-mute on|off] [--agc on|off]
//!                          [--dereverb off|small|medium|large]
//!                          [--lsnr-out lsnr.f32]`
//!
//! Files are raw mono 48 kHz f32le (convert with `ffmpeg -i in.wav -f f32le -ac 1 -ar 48000 in.f32`).
//! The output is shifted back by the pipeline latency so it lines up with the input.

use sordino_core::denoise::{Strength, Thresholds};
use sordino_core::pipeline::{Pipeline, PipelineParams};
use sordino_core::studio::Preset;
use sordino_core::HOP;

fn read(path: &str) -> std::io::Result<Vec<f32>> {
    Ok(std::fs::read(path)?
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 2,
        "usage: process_file in.f32 out.f32 [--noise ..] [--studio ..] [--echo ref.f32]"
    );
    let mut params = PipelineParams {
        auto_eq: true,
        echo: false,
        noise: true,
        strength: Strength::High,
        studio: Preset::Natural.params(),
        pause_mute: true,
        mute: false,
        agc: true,
        dereverb: None,
    };
    let mut reference: Option<Vec<f32>> = None;
    let mut thresholds = Thresholds::default();
    let mut lsnr_out: Option<String> = None;
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--noise" => match it.next().map(String::as_str) {
                Some("off") => params.noise = false,
                Some(s) => {
                    params.strength =
                        Strength::parse(s).ok_or_else(|| anyhow::anyhow!("bad strength {s}"))?
                }
                None => anyhow::bail!("--noise needs a value"),
            },
            "--studio" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--studio needs a value"))?;
                let p = Preset::parse(v).ok_or_else(|| anyhow::anyhow!("bad preset {v}"))?;
                params.studio = p.params();
            }
            "--autoeq" => match it.next().map(String::as_str) {
                Some("on") => params.auto_eq = true,
                Some("off") => params.auto_eq = false,
                _ => anyhow::bail!("--autoeq on|off"),
            },
            "--dereverb" => match it.next().map(String::as_str) {
                Some("off") => params.dereverb = None,
                Some(r) => {
                    params.dereverb = Some(
                        sordino_core::dereverb::RoomSize::parse(r)
                            .ok_or_else(|| anyhow::anyhow!("--dereverb off|small|medium|large"))?,
                    )
                }
                None => anyhow::bail!("--dereverb off|small|medium|large"),
            },
            "--agc" => match it.next().map(String::as_str) {
                Some("on") => params.agc = true,
                Some("off") => params.agc = false,
                _ => anyhow::bail!("--agc on|off"),
            },
            "--pause-mute" => match it.next().map(String::as_str) {
                Some("on") => params.pause_mute = true,
                Some("off") => params.pause_mute = false,
                _ => anyhow::bail!("--pause-mute on|off"),
            },
            "--thresh" => {
                let v: Vec<f32> = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--thresh needs min,erb,df"))?
                    .split(',')
                    .map(|x| x.parse())
                    .collect::<Result<_, _>>()?;
                anyhow::ensure!(v.len() == 3, "--thresh needs three values");
                thresholds = Thresholds {
                    min_db: v[0],
                    max_erb_db: v[1],
                    max_df_db: v[2],
                };
            }
            "--lsnr-out" => {
                lsnr_out = Some(
                    it.next()
                        .ok_or_else(|| anyhow::anyhow!("--lsnr-out needs a file"))?
                        .clone(),
                )
            }
            "--echo" => {
                reference = Some(read(
                    it.next()
                        .ok_or_else(|| anyhow::anyhow!("--echo needs a file"))?,
                )?);
                params.echo = true;
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }

    let input = read(&args[0])?;
    let mut p = Pipeline::with_thresholds(params, thresholds)?;
    let latency = p.latency_samples();
    let hops = input.len().div_ceil(HOP);
    let mut padded = input.clone();
    padded.resize(hops * HOP + latency.div_ceil(HOP) * HOP, 0.0);
    let mut out = vec![0.0f32; padded.len()];
    let mut lsnr = Vec::with_capacity(hops);
    for (i, (inp, o)) in padded
        .chunks_exact(HOP)
        .zip(out.chunks_exact_mut(HOP))
        .enumerate()
    {
        let r = reference
            .as_ref()
            .and_then(|r| r.get(i * HOP..(i + 1) * HOP));
        p.process(inp, r, o)?;
        lsnr.push(p.last_lsnr().unwrap_or(f32::NAN));
    }
    let aligned = &out[latency..latency + input.len()];
    std::fs::write(
        &args[1],
        aligned
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>(),
    )?;
    if let Some(path) = lsnr_out {
        // One value per hop of *input*, f32le.
        std::fs::write(
            path,
            lsnr.iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>(),
        )?;
    }
    if params.auto_eq {
        eprintln!(
            "auto EQ gains (dB) per band {:?}: {:?}",
            sordino_core::autoeq::BANDS,
            p.auto_eq_gains().map(|g| (g * 10.0).round() / 10.0)
        );
        let (prof, snr, hops) = p.auto_eq_debug();
        eprintln!(
            "  profile {:?}\n  band snr {:?}\n  speech hops {hops}",
            prof.map(|g| (g * 10.0).round() / 10.0),
            snr.map(|g| (g * 10.0).round() / 10.0)
        );
    }
    eprintln!(
        "processed {:.1} s, latency {} samples ({:.1} ms)",
        input.len() as f32 / 48000.0,
        latency,
        latency as f32 * 1000.0 / 48000.0
    );
    Ok(())
}
