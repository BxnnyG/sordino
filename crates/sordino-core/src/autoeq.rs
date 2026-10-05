//! Automatic microphone correction ("auto EQ").
//!
//! Cheap microphones often lose a lot above 4 kHz, which makes speech sound dull ("underwater").
//! A fixed preset cannot fix that for every mic. This stage measures the long-term spectrum of the
//! user's *speech* (only frames the noise model rates as clean speech), compares it with a
//! reference speech profile and applies a slow, bounded correction EQ.
//!
//! Safety rules: corrections move slowly (no audible pumping), are limited per band, and a band is
//! only boosted when it actually carries speech above its noise floor (boosting a band that holds
//! nothing but hiss would only add hiss).

use crate::dsp::biquad::{Biquad, Coeffs, FilterKind};
use crate::SAMPLE_RATE;

const FS: f32 = SAMPLE_RATE as f32;

/// Analysis band centres (Hz), one octave wide each.
pub const BANDS: [f32; 8] = [200.0, 400.0, 800.0, 1600.0, 3200.0, 5000.0, 7000.0, 10000.0];
/// Bands used as the 0 dB reference of a profile (400 Hz .. 3.2 kHz).
const REF_BANDS: std::ops::Range<usize> = 1..5;

/// Long-term speech profile of a studio recording, measured with this exact analyser
/// (`cargo run --release -p sordino-core --example speech_profile -- clean.f32`).
pub const TARGET_PROFILE: [f32; 8] = [1.3, 3.2, 0.4, -2.9, -5.1, -5.7, -4.9, -6.7];

/// (band index, maximum cut dB, maximum boost dB). Only these bands are corrected.
const CORRECTED: [(usize, f32, f32); 5] = [
    (0, -6.0, 2.0),
    (4, -4.0, 2.0),
    (5, -4.0, 8.0),
    (6, -4.0, 8.0),
    (7, -4.0, 5.0),
];

/// How many speech hops before corrections start (about 3 s of talking).
const WARMUP_HOPS: u32 = 300;
/// Correction is recomputed every this many hops (0.5 s) and moves at most `STEP_DB` each time.
const UPDATE_HOPS: u32 = 50;
const STEP_DB: f32 = 0.5;

pub struct Analyzer {
    filters: Vec<Biquad>,
    /// Exponential averages of band power in speech and in noise.
    speech: [f64; 8],
    noise: [f64; 8],
    speech_hops: u32,
    noise_hops: u32,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let filters = BANDS.iter().map(|&f| Biquad::new(band_coeffs(f))).collect();
        Analyzer {
            filters,
            speech: [0.0; 8],
            noise: [0.0; 8],
            speech_hops: 0,
            noise_hops: 0,
        }
    }

    /// Feed one hop. `speech` says whether the hop is clean speech, `noise_only` whether it is
    /// pure background; hops that are neither are only used to keep the filters running.
    pub fn feed(&mut self, hop: &[f32], speech: bool, noise_only: bool) {
        let mut power = [0.0f64; 8];
        for (b, f) in self.filters.iter_mut().enumerate() {
            let mut acc = 0.0f64;
            for &x in hop {
                let y = f.process_sample(x) as f64;
                acc += y * y;
            }
            power[b] = acc / hop.len() as f64;
        }
        if speech {
            self.speech_hops += 1;
            // Plain mean while warming up, then a ~60 s exponential window.
            let a = (1.0 / self.speech_hops as f64).max(1.0 / 6000.0);
            for (avg, p) in self.speech.iter_mut().zip(power) {
                *avg += a * (p - *avg);
            }
        } else if noise_only {
            self.noise_hops += 1;
            let a = (1.0 / self.noise_hops as f64).max(1.0 / 3000.0);
            for (avg, p) in self.noise.iter_mut().zip(power) {
                *avg += a * (p - *avg);
            }
        }
    }

    pub fn speech_hops(&self) -> u32 {
        self.speech_hops
    }

    /// Speech profile in dB relative to the 400 Hz .. 3.2 kHz bands.
    pub fn profile(&self) -> [f32; 8] {
        let reference = REF_BANDS.map(|b| self.speech[b]).sum::<f64>() / REF_BANDS.len() as f64;
        self.speech
            .map(|s| (10.0 * ((s + 1e-20) / (reference + 1e-20)).log10()) as f32)
    }

    /// Speech-to-noise ratio per band in dB (how much a band carries above its background).
    pub fn band_snr(&self) -> [f32; 8] {
        if self.noise_hops < 50 {
            return [60.0; 8];
        }
        std::array::from_fn(|b| {
            (10.0 * ((self.speech[b] + 1e-20) / (self.noise[b] + 1e-20)).log10()) as f32
        })
    }
}

fn band_coeffs(f: f32) -> Coeffs {
    // Q 1.41 is roughly one octave.
    Coeffs::new(FilterKind::BandPass, f, 1.41, 0.0, FS)
}

pub struct AutoEq {
    analyzer: Analyzer,
    gains: [f32; 8],
    eq: Vec<(usize, Biquad)>,
    hops: u32,
}

impl Default for AutoEq {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoEq {
    /// Diagnostics: speech profile, per-band SNR and number of speech hops seen so far.
    pub fn debug(&self) -> ([f32; 8], [f32; 8], u32) {
        (
            self.analyzer.profile(),
            self.analyzer.band_snr(),
            self.analyzer.speech_hops(),
        )
    }

    pub fn new() -> Self {
        let eq = CORRECTED
            .iter()
            .map(|&(b, _, _)| (b, Biquad::new(Coeffs::PASS)))
            .collect();
        AutoEq {
            analyzer: Analyzer::new(),
            gains: [0.0; 8],
            eq,
            hops: 0,
        }
    }

    /// Current correction per analysis band (dB), for display and tests.
    pub fn gains(&self) -> [f32; 8] {
        self.gains
    }

    /// The target correction for the current measurement (before rate limiting).
    fn wanted(&self) -> [f32; 8] {
        let mut want = [0.0f32; 8];
        if self.analyzer.speech_hops() < WARMUP_HOPS {
            return want;
        }
        let profile = self.analyzer.profile();
        let snr = self.analyzer.band_snr();
        for &(b, cut, boost) in &CORRECTED {
            let mut g = (TARGET_PROFILE[b] - profile[b]).clamp(cut, boost);
            // Never boost a band that is mostly background noise.
            if g > 0.0 {
                let headroom = (snr[b] - 6.0).max(0.0);
                g = g.min(headroom);
            }
            want[b] = g;
        }
        want
    }

    /// Analyse and correct one hop in place. `lsnr` is the noise model's local SNR estimate in dB
    /// for this hop (None when noise suppression is off: the correction then stays as it is).
    pub fn process(&mut self, hop: &mut [f32], lsnr: Option<f32>) {
        if let Some(lsnr) = lsnr {
            let rms = (hop.iter().map(|x| x * x).sum::<f32>() / hop.len() as f32).sqrt();
            let loud = rms > 1e-3; // about -60 dBFS
            self.analyzer.feed(hop, lsnr > 15.0 && loud, lsnr < 0.0);
        }
        self.hops += 1;
        if self.hops % UPDATE_HOPS == 0 {
            let want = self.wanted();
            for &(b, _, _) in &CORRECTED {
                let d = (want[b] - self.gains[b]).clamp(-STEP_DB, STEP_DB);
                self.gains[b] += d;
            }
            for (b, f) in &mut self.eq {
                let g = self.gains[*b];
                let c = if g.abs() < 0.05 {
                    Coeffs::PASS
                } else if *b == 0 {
                    Coeffs::new(FilterKind::LowShelf, 250.0, 0.707, g, FS)
                } else if *b == 7 {
                    Coeffs::new(FilterKind::HighShelf, 8500.0, 0.707, g, FS)
                } else {
                    Coeffs::new(FilterKind::Peaking, BANDS[*b], 1.0, g, FS)
                };
                f.set_coeffs(c);
            }
        }
        for (_, f) in &mut self.eq {
            f.process(hop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pink-ish "speech" with a controllable high-frequency deficit.
    fn shaped_noise(n: usize, hf_gain_db: f32, seed: u32) -> Vec<f32> {
        let mut s = seed;
        let mut white: Vec<f32> = (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 8) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect();
        // Tilt like speech (lowpass) and apply the deficit as a high shelf.
        let mut lp = Biquad::new(Coeffs::new(FilterKind::LowPass, 2500.0, 0.6, 0.0, FS));
        let mut shelf = Biquad::new(Coeffs::new(
            FilterKind::HighShelf,
            5000.0,
            0.707,
            hf_gain_db,
            FS,
        ));
        lp.process(&mut white);
        shelf.process(&mut white);
        white.iter().map(|x| x * 0.2).collect()
    }

    #[test]
    fn boosts_a_dull_microphone_and_stays_bounded() {
        let mut eq = AutoEq::new();
        let x = shaped_noise(48000 * 40, -15.0, 1);
        let mut y = x.clone();
        for hop in y.chunks_exact_mut(crate::HOP) {
            eq.process(hop, Some(25.0)); // pretend it is all clean speech
        }
        let g = eq.gains();
        assert!(g[6] > 2.0 && g[7] > 2.0, "highs should be boosted: {g:?}");
        for &(b, cut, boost) in &CORRECTED {
            assert!(
                g[b] >= cut - 1e-3 && g[b] <= boost + 1e-3,
                "band {b} out of bounds: {g:?}"
            );
        }
        assert!(y.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn does_nothing_without_enough_speech() {
        let mut eq = AutoEq::new();
        let x = shaped_noise(48000 * 2, -15.0, 2);
        let mut y = x.clone();
        for hop in y.chunks_exact_mut(crate::HOP) {
            eq.process(hop, Some(25.0));
        }
        assert!(eq.gains().iter().all(|g| *g == 0.0));
        assert_eq!(x, y, "no correction before warm-up must be bit-exact");
    }

    #[test]
    fn never_boosts_bands_that_only_hold_noise() {
        let mut eq = AutoEq::new();
        // "Speech" and background have the same spectrum: no band has speech above noise.
        let x = shaped_noise(48000 * 40, -15.0, 3);
        let mut y = x.clone();
        for (i, hop) in y.chunks_exact_mut(crate::HOP).enumerate() {
            let lsnr = if i % 2 == 0 { 25.0 } else { -5.0 };
            eq.process(hop, Some(lsnr));
        }
        assert!(
            eq.gains().iter().all(|g| *g <= 0.0 + 1e-3),
            "{:?}",
            eq.gains()
        );
    }
}
