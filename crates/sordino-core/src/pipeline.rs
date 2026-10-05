//! Echo cancel -> denoise -> studio chain -> pause mute, one hop at a time, with click-free on/off
//! switching.

use anyhow::Result;

use crate::autoeq::AutoEq;
use crate::denoise::{Denoiser, Strength, Thresholds};
use crate::echo::Echo;
use crate::speech_gate::SpeechGate;
use crate::studio::{StudioChain, StudioParams};
use crate::HOP;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PipelineParams {
    pub echo: bool,
    pub noise: bool,
    pub strength: Strength,
    /// Automatic microphone correction (see [`crate::autoeq`]).
    pub auto_eq: bool,
    /// `None` = studio sound off.
    pub studio: Option<StudioParams>,
    /// Mute between words (see [`crate::speech_gate`]). Only acts while noise suppression runs.
    pub pause_mute: bool,
    /// Send silence (mute button). Fades over one hop.
    pub mute: bool,
}

impl Default for PipelineParams {
    fn default() -> Self {
        PipelineParams {
            echo: false,
            noise: true,
            strength: Strength::High,
            auto_eq: true,
            studio: StudioParams::default().into(),
            pause_mute: true,
            mute: false,
        }
    }
}

pub struct Pipeline {
    echo: Option<Echo>,
    denoiser: Denoiser,
    studio: StudioChain,
    auto_eq: AutoEq,
    gate: SpeechGate,
    params: PipelineParams,
    /// Current wet/dry mix (0 = dry, 1 = processed) of each stage, ramped over one hop.
    echo_mix: f32,
    noise_mix: f32,
    studio_mix: f32,
    /// 1 = audible, 0 = muted.
    mute_gain: f32,
    stage: [f32; HOP],
    scratch: [f32; HOP],
    /// The caller is falling behind real time: skip the expensive noise stage until it catches up.
    overloaded: bool,
    /// The noise model's local SNR estimate of the last hop (None while the model is off).
    last_lsnr: Option<f32>,
}

const SILENCE: [f32; HOP] = [0.0; HOP];

impl Pipeline {
    pub fn new(params: PipelineParams) -> Result<Self> {
        Self::with_thresholds(params, Thresholds::default())
    }

    pub fn with_thresholds(params: PipelineParams, thresholds: Thresholds) -> Result<Self> {
        let denoiser = Denoiser::with_thresholds(params.strength, thresholds)?;
        let studio = StudioChain::new(params.studio.unwrap_or_default());
        // A missing echo canceller is not fatal: everything else keeps working.
        let echo = match Echo::new() {
            Ok(e) => Some(e),
            Err(e) => {
                log::info!("echo cancellation unavailable: {e}");
                None
            }
        };
        Ok(Pipeline {
            echo,
            denoiser,
            studio,
            params,
            echo_mix: 0.0,
            noise_mix: if params.noise { 1.0 } else { 0.0 },
            studio_mix: if params.studio.is_some() { 1.0 } else { 0.0 },
            mute_gain: if params.mute { 0.0 } else { 1.0 },
            stage: [0.0; HOP],
            scratch: [0.0; HOP],
            overloaded: false,
            last_lsnr: None,
            auto_eq: AutoEq::new(),
            gate: SpeechGate::new(),
        })
    }

    /// Tell the pipeline that the caller is behind real time. While set, the noise stage is
    /// bypassed (with the usual crossfade) so the backlog can be worked off cheaply.
    pub fn set_overloaded(&mut self, overloaded: bool) {
        self.overloaded = overloaded;
    }

    pub fn params(&self) -> PipelineParams {
        self.params
    }

    /// Current automatic correction per band (dB), see [`crate::autoeq::BANDS`].
    pub fn auto_eq_gains(&self) -> [f32; 8] {
        self.auto_eq.gains()
    }

    /// Diagnostics of the automatic correction: (profile, per-band SNR, speech hops).
    pub fn auto_eq_debug(&self) -> ([f32; 8], [f32; 8], u32) {
        self.auto_eq.debug()
    }

    /// The noise model's local SNR estimate (dB) of the last processed hop.
    pub fn last_lsnr(&self) -> Option<f32> {
        self.last_lsnr
    }

    /// Debug summary of the echo canceller, if there is one.
    pub fn echo_stats(&self) -> Option<String> {
        self.echo.as_ref().map(Echo::stats)
    }

    /// Whether echo cancellation can be switched on.
    pub fn echo_available(&self) -> bool {
        self.echo.is_some()
    }

    pub fn set_params(&mut self, p: PipelineParams) {
        if p.strength != self.params.strength {
            self.denoiser.set_strength(p.strength);
        }
        if let Some(s) = p.studio {
            self.studio.set_params(s);
            if self.studio_mix == 0.0 {
                // Coming back from "off": do not start from stale filter state.
                self.studio.reset();
            }
        }
        self.params = p;
    }

    /// Latency in samples of the currently active stages.
    pub fn latency_samples(&self) -> usize {
        let noise = if self.params.noise {
            self.denoiser.latency_samples()
        } else {
            0
        };
        let studio = if self.params.studio.is_some() {
            self.studio.latency()
        } else {
            0
        };
        noise + studio
    }

    /// Process one hop. `reference` is what is currently being played (for the echo canceller);
    /// pass `None` when there is none. `output` is always written: if a stage fails, its input is
    /// passed through and the error is returned so the caller can report it.
    pub fn process(
        &mut self,
        input: &[f32],
        reference: Option<&[f32]>,
        output: &mut [f32],
    ) -> Result<()> {
        debug_assert_eq!(input.len(), HOP);
        debug_assert_eq!(output.len(), HOP);
        self.stage.copy_from_slice(input);
        let mut result = Ok(());

        // Stage 0: echo cancellation.
        let echo_target = if self.params.echo && self.echo.is_some() {
            1.0
        } else {
            0.0
        };
        if self.echo_mix > 0.0 || echo_target > 0.0 {
            if let Some(echo) = self.echo.as_mut() {
                self.scratch.copy_from_slice(&self.stage);
                match echo.process(reference.unwrap_or(&SILENCE), &mut self.scratch) {
                    Ok(()) => crossfade(&mut self.stage, &self.scratch, self.echo_mix, echo_target),
                    Err(e) => result = Err(e),
                }
            }
            self.echo_mix = echo_target;
        }

        // The pause gate looks at the signal *before* the noise model: the model's delay is the
        // gate's look-ahead (see [`crate::speech_gate`]).
        if self.params.pause_mute {
            self.gate.analyze(&self.stage);
        }

        // Stage 1: noise suppression.
        let noise_target = if self.params.noise && !self.overloaded {
            1.0
        } else {
            0.0
        };
        let mut lsnr = None;
        if self.noise_mix > 0.0 || noise_target > 0.0 {
            match self.denoiser.process_hop(&self.stage, &mut self.scratch) {
                Ok(l) => {
                    crossfade(&mut self.stage, &self.scratch, self.noise_mix, noise_target);
                    if noise_target > 0.0 {
                        lsnr = Some(l);
                    }
                }
                Err(e) => result = Err(e),
            }
            self.noise_mix = noise_target;
        }

        self.last_lsnr = lsnr;

        // Stage 1b: automatic microphone correction. It learns only while the noise model runs
        // (it needs the model's speech/noise decision) and keeps its last correction otherwise.
        if self.params.auto_eq {
            self.auto_eq.process(&mut self.stage, lsnr);
        }

        // Stage 2: studio chain.
        let studio_target = if self.params.studio.is_some() {
            1.0
        } else {
            0.0
        };
        if self.studio_mix > 0.0 || studio_target > 0.0 {
            self.scratch.copy_from_slice(&self.stage);
            self.studio.process(&mut self.scratch);
            crossfade(
                &mut self.stage,
                &self.scratch,
                self.studio_mix,
                studio_target,
            );
            self.studio_mix = studio_target;
        }

        // Stage 3: mute between words. It relies on the noise model's delay, so it only acts
        // while that stage runs.
        self.gate.apply(
            &mut self.stage,
            self.params.pause_mute && noise_target > 0.0,
        );

        // Mute button.
        let mute_target = if self.params.mute { 0.0 } else { 1.0 };
        if self.mute_gain != 1.0 || mute_target != 1.0 {
            crossfade(
                &mut self.stage,
                &SILENCE,
                1.0 - self.mute_gain,
                1.0 - mute_target,
            );
            self.mute_gain = mute_target;
        }

        // Never hand NaN/inf to the graph (a single one can poison downstream filters).
        for (o, s) in output.iter_mut().zip(self.stage.iter()) {
            *o = if s.is_finite() { *s } else { 0.0 };
        }
        result
    }
}

/// In place: `dry = dry + (wet - dry) * mix`, with `mix` ramping linearly from `from` to `to`.
fn crossfade(dry: &mut [f32], wet: &[f32], from: f32, to: f32) {
    let n = dry.len() as f32;
    for (i, d) in dry.iter_mut().enumerate() {
        let m = from + (to - from) * (i as f32 + 1.0) / n;
        *d += (wet[i] - *d) * m;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::rms_db;

    fn tone(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.2 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin())
            .collect()
    }

    fn run(p: &mut Pipeline, x: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0; x.len()];
        for (i, o) in x.chunks_exact(HOP).zip(out.chunks_exact_mut(HOP)) {
            p.process(i, None, o).unwrap();
        }
        out
    }

    fn off() -> PipelineParams {
        PipelineParams {
            echo: false,
            noise: false,
            strength: Strength::High,
            auto_eq: false,
            studio: None,
            pause_mute: false,
            mute: false,
        }
    }

    #[test]
    fn everything_off_is_bit_exact_passthrough() {
        let mut p = Pipeline::new(off()).unwrap();
        let x = tone(HOP * 20);
        assert_eq!(run(&mut p, &x), x);
        assert_eq!(p.latency_samples(), 0);
    }

    #[test]
    fn toggling_noise_has_no_hard_discontinuity() {
        let mut p = Pipeline::new(PipelineParams {
            noise: true,
            ..off()
        })
        .unwrap();
        let x = tone(HOP * 100);
        let mut out = run(&mut p, &x[..HOP * 50]);
        let mut params = p.params();
        params.noise = false;
        p.set_params(params);
        out.extend(run(&mut p, &x[HOP * 50..]));
        // DeepFilterNet's delay makes dry/wet differ in phase, so require only that the
        // switch does not produce a spike beyond what the signal itself could.
        assert!(out.iter().all(|v| v.abs() < 0.6));
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn output_is_always_finite_and_studio_changes_loudness_bounded() {
        let mut p = Pipeline::new(PipelineParams::default()).unwrap();
        let x = tone(HOP * 100);
        let out = run(&mut p, &x);
        assert!(out.iter().all(|v| v.is_finite()));
        assert!(rms_db(&out) < 0.0);
        assert!(p.latency_samples() > 0);
    }

    #[test]
    fn mute_fades_to_silence_and_back() {
        let mut p = Pipeline::new(off()).unwrap();
        let x = tone(HOP * 10);
        let mut params = p.params();
        params.mute = true;
        p.set_params(params);
        let y = run(&mut p, &x);
        assert!(
            y[HOP..].iter().all(|v| *v == 0.0),
            "silent after the fade hop"
        );
        assert!(y[..HOP].iter().all(|v| v.abs() <= 0.2 + 1e-6));
        params.mute = false;
        p.set_params(params);
        let z = run(&mut p, &x);
        assert_eq!(&z[HOP..], &x[HOP..], "bit-exact again after the fade hop");
    }

    #[test]
    fn pause_mute_needs_the_noise_stage() {
        // Without the noise model's delay the gate has no look-ahead, so it stays out of the way.
        let mut p = Pipeline::new(PipelineParams {
            pause_mute: true,
            ..off()
        })
        .unwrap();
        let x = tone(HOP * 50);
        assert_eq!(run(&mut p, &x), x);
    }

    #[test]
    fn echo_flag_without_canceller_is_harmless() {
        let mut p = Pipeline::new(PipelineParams {
            echo: true,
            ..off()
        })
        .unwrap();
        let x = tone(HOP * 10);
        let out = run(&mut p, &x);
        if !p.echo_available() {
            assert_eq!(out, x);
        }
        assert!(out.iter().all(|v| v.is_finite()));
    }
}

#[cfg(test)]
mod overload_tests {
    use super::*;

    #[test]
    fn overload_bypasses_the_noise_stage_and_recovers() {
        let mut p = Pipeline::new(PipelineParams {
            echo: false,
            noise: true,
            strength: Strength::High,
            auto_eq: false,
            studio: None,
            pause_mute: false,
            mute: false,
        })
        .unwrap();
        let x: Vec<f32> = (0..HOP * 40)
            .map(|i| 0.2 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / 48000.0).sin())
            .collect();
        let mut out = vec![0.0; x.len()];
        p.set_overloaded(true);
        for (i, o) in x.chunks_exact(HOP).zip(out.chunks_exact_mut(HOP)).take(20) {
            p.process(i, None, o).unwrap();
        }
        // While overloaded the signal passes through untouched (after the fade-out hop).
        assert_eq!(&out[HOP * 2..HOP * 20], &x[HOP * 2..HOP * 20]);
        p.set_overloaded(false);
        for (i, o) in x.chunks_exact(HOP).zip(out.chunks_exact_mut(HOP)).skip(20) {
            p.process(i, None, o).unwrap();
        }
        assert!(out.iter().all(|v| v.is_finite()));
    }
}
