//! Room echo ("reverb") reduction.
//!
//! In a bare room your voice keeps ringing for a few hundred milliseconds after each word, which
//! makes you sound far away. This stage estimates that late reverberation and suppresses it,
//! following the classic statistical model (Lebart et al. 2001, Habets 2007): in a room with
//! reverberation time T60 the sound energy decays by 60 dB in T60 seconds, so the late reverb in
//! a frequency bin now is roughly the bin's power `Td` seconds ago, attenuated by that decay.
//! Subtracting it (as a spectral gain with a floor) removes the tail but keeps the direct sound.
//!
//! The room size comes from the strength setting instead of a blind T60 estimate: estimates are
//! unreliable on short speech and a wrong one either does nothing or eats into the voice.
//!
//! Short-time Fourier transform: 20 ms sqrt-Hann frames with 10 ms hop (one Sordino hop), which
//! adds 10 ms latency while the stage is on.

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::{HOP, SAMPLE_RATE};

const N: usize = 2 * HOP;
const BINS: usize = N / 2 + 1;
/// Late reverberation starts this many hops after the direct sound (50 ms).
const DELAY_HOPS: usize = 5;
/// Recursive smoothing of the power spectrum.
const PSD_SMOOTH: f32 = 0.6;
/// Gain smoothing over time against "musical noise".
const GAIN_SMOOTH: f32 = 0.5;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoomSize {
    /// Normal furnished room.
    #[default]
    Small,
    /// Larger or sparsely furnished room.
    Medium,
    /// Bare room, hall.
    Large,
}

impl RoomSize {
    pub const ALL: [RoomSize; 3] = [RoomSize::Small, RoomSize::Medium, RoomSize::Large];

    /// Assumed reverberation time (s) and the deepest cut (dB).
    fn params(self) -> (f32, f32) {
        match self {
            RoomSize::Small => (0.35, -9.0),
            RoomSize::Medium => (0.6, -12.0),
            RoomSize::Large => (0.9, -15.0),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RoomSize::Small => "small",
            RoomSize::Medium => "medium",
            RoomSize::Large => "large",
        }
    }

    pub fn parse(s: &str) -> Option<RoomSize> {
        RoomSize::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

pub struct Dereverb {
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    window: Vec<f32>,
    /// Last N input samples.
    input: Vec<f32>,
    /// Overlap-add buffer.
    overlap: Vec<f32>,
    frame: Vec<f32>,
    spectrum: Vec<Complex32>,
    scratch_fwd: Vec<Complex32>,
    scratch_inv: Vec<Complex32>,
    /// Smoothed power spectra of the last `DELAY_HOPS + 1` frames (ring).
    history: Vec<[f32; BINS]>,
    pos: usize,
    gain: [f32; BINS],
    /// Energy decay over the delay, and the gain floor (linear).
    decay: f32,
    floor: f32,
}

impl Dereverb {
    pub fn new(room: RoomSize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(N);
        let ifft = planner.plan_fft_inverse(N);
        // sqrt-Hann for analysis and synthesis: their product (Hann) adds up to 1 at 50 % overlap.
        let window = (0..N)
            .map(|i| (std::f32::consts::PI * (i as f32 + 0.5) / N as f32).sin())
            .collect();
        let scratch_fwd = fft.make_scratch_vec();
        let scratch_inv = ifft.make_scratch_vec();
        let mut d = Dereverb {
            spectrum: fft.make_output_vec(),
            fft,
            ifft,
            window,
            input: vec![0.0; N],
            overlap: vec![0.0; HOP],
            frame: vec![0.0; N],
            scratch_fwd,
            scratch_inv,
            history: vec![[0.0; BINS]; DELAY_HOPS + 1],
            pos: 0,
            gain: [1.0; BINS],
            decay: 0.0,
            floor: 0.0,
        };
        d.set_room(room);
        d
    }

    pub fn set_room(&mut self, room: RoomSize) {
        let (t60, floor_db) = room.params();
        // Energy decays by 60 dB per T60: over `Td` seconds by 10^(-6 Td / T60).
        let td = (DELAY_HOPS * HOP) as f32 / SAMPLE_RATE as f32;
        self.decay = 10f32.powf(-6.0 * td / t60);
        self.floor = 10f32.powf(floor_db / 20.0);
    }

    /// Added latency in samples.
    pub const fn latency() -> usize {
        HOP
    }

    pub fn reset(&mut self) {
        self.input.fill(0.0);
        self.overlap.fill(0.0);
        for h in &mut self.history {
            h.fill(0.0);
        }
        self.gain = [1.0; BINS];
    }

    /// Process one hop in place (output is delayed by [`Self::latency`]).
    pub fn process(&mut self, hop: &mut [f32]) {
        debug_assert_eq!(hop.len(), HOP);
        self.input.copy_within(HOP.., 0);
        self.input[N - HOP..].copy_from_slice(hop);
        for ((f, x), w) in self.frame.iter_mut().zip(&self.input).zip(&self.window) {
            *f = x * w;
        }
        if self
            .fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch_fwd)
            .is_err()
        {
            return;
        }

        // Smoothed power of this frame, and the late-reverb estimate from DELAY_HOPS ago.
        let prev = self.history[self.pos];
        self.pos = (self.pos + 1) % self.history.len();
        let old = self.history[self.pos]; // oldest frame: DELAY_HOPS before the newest
        let mut now = [0.0f32; BINS];
        for k in 0..BINS {
            let p = self.spectrum[k].norm_sqr();
            now[k] = PSD_SMOOTH * prev[k] + (1.0 - PSD_SMOOTH) * p;
            let late = self.decay * old[k];
            let g = (1.0 - late / (now[k] + 1e-12))
                .max(self.floor * self.floor)
                .sqrt();
            self.gain[k] = GAIN_SMOOTH * self.gain[k] + (1.0 - GAIN_SMOOTH) * g;
            self.spectrum[k] *= self.gain[k];
        }
        self.history[self.pos] = now;
        // DC and Nyquist must be real for the inverse transform.
        self.spectrum[0].im = 0.0;
        self.spectrum[BINS - 1].im = 0.0;

        if self
            .ifft
            .process_with_scratch(&mut self.spectrum, &mut self.frame, &mut self.scratch_inv)
            .is_err()
        {
            return;
        }
        let scale = 1.0 / N as f32;
        for (i, out) in hop.iter_mut().enumerate() {
            *out = self.overlap[i] + self.frame[i] * self.window[i] * scale;
        }
        for i in 0..HOP {
            self.overlap[i] = self.frame[HOP + i] * self.window[HOP + i] * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(d: &mut Dereverb, x: &[f32]) -> Vec<f32> {
        let mut y = x.to_vec();
        for hop in y.chunks_exact_mut(HOP) {
            d.process(hop);
        }
        y
    }

    fn noise(n: usize, seed: &mut u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (*seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect()
    }

    /// Bursts of "speech" (noise) through a synthetic room with an exponential tail.
    fn reverberant(t60: f32) -> (Vec<f32>, Vec<f32>) {
        let mut seed = 3;
        let fs = SAMPLE_RATE as f32;
        let mut dry = vec![0.0; HOP * 420];
        for b in 0..8 {
            let start = b * HOP * 50;
            let burst = noise(HOP * 20, &mut seed);
            dry[start..start + HOP * 20].copy_from_slice(&burst);
        }
        let len = (t60 * fs) as usize;
        let rir_noise = noise(len, &mut seed);
        let rir: Vec<f32> = (0..len)
            .map(|i| {
                if i == 0 {
                    1.0
                } else {
                    0.3 * rir_noise[i] * 10f32.powf(-3.0 * i as f32 / (t60 * fs))
                }
            })
            .collect();
        let mut wet = vec![0.0; dry.len()];
        for (i, &x) in dry.iter().enumerate().filter(|(_, x)| **x != 0.0) {
            for (j, &h) in rir.iter().enumerate().take(wet.len() - i) {
                wet[i + j] += x * h;
            }
        }
        (dry, wet)
    }

    #[test]
    fn passes_a_dry_signal_almost_unchanged() {
        // A steady tone looks like its own reverb to the model, so it loses the decay ratio
        // (about 1.6 dB for a medium room) but must come through clean otherwise.
        let x: Vec<f32> = (0..HOP * 200)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / 48000.0).sin())
            .collect();
        let mut d = Dereverb::new(RoomSize::Medium);
        let y = run(&mut d, &x);
        let lat = Dereverb::latency();
        let err: f32 = y[HOP * 50..]
            .iter()
            .zip(&x[HOP * 50 - lat..])
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            / x[HOP * 50..].iter().map(|v| v * v).sum::<f32>();
        assert!(err < 0.05, "relative error {err}");
    }

    #[test]
    fn shortens_the_reverb_tail() {
        let (dry, wet) = reverberant(0.6);
        let mut d = Dereverb::new(RoomSize::Medium);
        let y = run(&mut d, &wet);
        let lat = Dereverb::latency();
        // Energy in the tails (100..400 ms after each burst) vs during the bursts.
        let tail = |s: &[f32], off: usize| -> f32 {
            (0..8)
                .map(|b| {
                    let t = b * HOP * 50 + HOP * 30 + off;
                    s[t..t + HOP * 20].iter().map(|v| v * v).sum::<f32>()
                })
                .sum()
        };
        let body = |s: &[f32], off: usize| -> f32 {
            (0..8)
                .map(|b| {
                    let t = b * HOP * 50 + HOP * 5 + off;
                    s[t..t + HOP * 10].iter().map(|v| v * v).sum::<f32>()
                })
                .sum()
        };
        let before = 10.0 * (tail(&wet, 0) / body(&wet, 0)).log10();
        let after = 10.0 * (tail(&y, lat) / body(&y, lat)).log10();
        assert!(
            after < before - 5.0,
            "tail/body {before:.1} dB -> {after:.1} dB"
        );
        assert!(dry.iter().all(|v| v.is_finite()) && y.iter().all(|v| v.is_finite()));
    }
}
