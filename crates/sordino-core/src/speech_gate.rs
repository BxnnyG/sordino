//! "Mute between words": silence the output while the user is not speaking.
//!
//! The noise model removes most background noise but lets short impulsive sounds (low keyboard
//! "thocks") through, especially between words. This gate opens only on *voiced speech*: a stable
//! pitch for at least two hops, well above the background level. Key clicks ring briefly but
//! rarely keep a stable pitch that long.
//!
//! No added latency: the analysis runs on the pipeline *input*, while the gain is applied to the
//! output, which the noise model has already delayed by about 30 ms. That delay is the gate's
//! look-ahead, so word onsets are not clipped. After the last voiced hop the gate holds (400 ms by
//! default, adjustable) for word endings and unvoiced consonants, then fades pauses down to
//! silence or, if the user prefers, only by a few dB (see [`GateParams`]).

use serde::{Deserialize, Serialize};

use crate::dsp::biquad::{Biquad, Coeffs, FilterKind};
use crate::{HOP, SAMPLE_RATE};

const FS: f32 = SAMPLE_RATE as f32;
/// The pitch analysis runs at 8 kHz (speech fundamentals are 80 .. 400 Hz).
const DECIM: usize = 6;
const HOP_D: usize = HOP / DECIM;
/// Analysis window: the last three hops (30 ms).
const WIN: usize = 3 * HOP_D;
/// Pitch lags at 8 kHz: 400 Hz .. 80 Hz.
const MIN_LAG: usize = 20;
const MAX_LAG: usize = 100;
const PITCH_TOLERANCE: f32 = 0.2;

/// How readily the gate opens.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    /// Opens only on clear voice: mutes the most typing, may miss very quiet speech.
    Low,
    #[default]
    Normal,
    /// Opens on quiet or breathy speech too; lets a little more typing through.
    High,
}

impl Sensitivity {
    pub const ALL: [Sensitivity; 3] = [Sensitivity::Low, Sensitivity::Normal, Sensitivity::High];

    /// (normalised autocorrelation a voiced hop needs, dB above the background, consecutive
    /// voiced hops with a stable pitch that open the gate)
    fn thresholds(self) -> (f32, f32, u32) {
        match self {
            Sensitivity::Low => (0.75, 12.0, 3),
            Sensitivity::Normal => (0.65, 8.0, 2),
            Sensitivity::High => (0.55, 6.0, 1),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Sensitivity::Low => "low",
            Sensitivity::Normal => "normal",
            Sensitivity::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Sensitivity> {
        Sensitivity::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// User settings of the gate.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(default)]
pub struct GateParams {
    /// How long the gate stays open after the last voiced sound (ms).
    pub hold_ms: u32,
    pub sensitivity: Sensitivity,
    /// How much pauses are lowered (dB, negative). At or below `MUTE_DB` pauses are silent.
    pub depth_db: f32,
}

impl GateParams {
    pub const MIN_HOLD_MS: u32 = 100;
    pub const MAX_HOLD_MS: u32 = 1500;
    pub const MUTE_DB: f32 = -60.0;

    pub fn sanitized(self) -> GateParams {
        GateParams {
            hold_ms: self.hold_ms.clamp(Self::MIN_HOLD_MS, Self::MAX_HOLD_MS),
            sensitivity: self.sensitivity,
            depth_db: if self.depth_db.is_finite() {
                self.depth_db.clamp(Self::MUTE_DB, -3.0)
            } else {
                Self::MUTE_DB
            },
        }
    }

    fn closed_gain(self) -> f32 {
        if self.depth_db <= Self::MUTE_DB {
            0.0
        } else {
            10f32.powf(self.depth_db / 20.0)
        }
    }
}

impl Default for GateParams {
    fn default() -> Self {
        GateParams {
            hold_ms: 400,
            sensitivity: Sensitivity::Normal,
            depth_db: Self::MUTE_DB,
        }
    }
}
/// The background level estimate follows quieter hops at once and rises at 0.5 dB/s.
const FLOOR_RISE_DB: f32 = 0.005;
const FLOOR_MIN_DB: f32 = -80.0;

pub struct SpeechGate {
    highpass: Biquad,
    lowpass: [Biquad; 2],
    /// Decimated, band-limited input, oldest first.
    window: [f32; WIN],
    floor_db: f32,
    run: u32,
    last_lag: usize,
    hold: u32,
    /// Current gain, smoothed per sample.
    gain: f32,
    attack: f32,
    release: f32,
    params: GateParams,
}

impl Default for SpeechGate {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechGate {
    pub fn new() -> Self {
        let lp = || Biquad::new(Coeffs::new(FilterKind::LowPass, 1000.0, 0.707, 0.0, FS));
        SpeechGate {
            highpass: Biquad::new(Coeffs::new(FilterKind::HighPass, 70.0, 0.707, 0.0, FS)),
            lowpass: [lp(), lp()],
            window: [0.0; WIN],
            floor_db: f32::INFINITY,
            run: 0,
            last_lag: 0,
            hold: 0,
            gain: 1.0,
            attack: 1.0 - (-1.0 / (0.005 * FS)).exp(),
            release: 1.0 - (-1.0 / (0.025 * FS)).exp(),
            params: GateParams::default(),
        }
    }

    pub fn set_params(&mut self, p: GateParams) {
        self.params = p.sanitized();
    }

    /// Whether the gate currently lets the voice through.
    pub fn is_open(&self) -> bool {
        self.hold > 0
    }

    /// Analyse one hop of the pipeline input (before the noise model).
    pub fn analyze(&mut self, input: &[f32]) {
        debug_assert_eq!(input.len(), HOP);
        let power = input.iter().map(|x| x * x).sum::<f32>() / HOP as f32;
        let level = (10.0 * (power + 1e-12).log10()).max(FLOOR_MIN_DB);
        self.floor_db = if level < self.floor_db {
            level
        } else {
            self.floor_db + FLOOR_RISE_DB
        };

        self.window.copy_within(HOP_D.., 0);
        for (k, chunk) in input.chunks_exact(DECIM).enumerate() {
            let mut y = 0.0;
            for &x in chunk {
                let band = self.lowpass[0].process_sample(self.highpass.process_sample(x));
                y += self.lowpass[1].process_sample(band);
            }
            // Averaging the six samples adds a little more anti-aliasing.
            self.window[WIN - HOP_D + k] = y / DECIM as f32;
        }

        let (periodicity_needed, above_floor, open_hops) = self.params.sensitivity.thresholds();
        let voiced = level > self.floor_db + above_floor;
        let (periodicity, lag) = if voiced {
            pitch(&self.window)
        } else {
            (0.0, 0)
        };
        if periodicity > periodicity_needed {
            let stable = self.run > 0
                && (lag as f32 - self.last_lag as f32).abs()
                    <= PITCH_TOLERANCE * self.last_lag as f32;
            self.run = if stable { self.run + 1 } else { 1 };
            self.last_lag = lag;
        } else {
            self.run = 0;
        }
        if self.run >= open_hops {
            self.hold = self.params.hold_ms.div_ceil(10);
        } else {
            self.hold = self.hold.saturating_sub(1);
        }
    }

    /// Apply the gate to one output hop. While `active` is false the gain returns smoothly to 1.
    pub fn apply(&mut self, output: &mut [f32], active: bool) {
        let target = if !active || self.is_open() {
            1.0
        } else {
            self.params.closed_gain()
        };
        if target == self.gain {
            for x in output.iter_mut() {
                *x *= target;
            }
            return;
        }
        let rate = if target > self.gain {
            self.attack
        } else {
            self.release
        };
        for x in output.iter_mut() {
            self.gain += (target - self.gain) * rate;
            *x *= self.gain;
        }
        if (self.gain - target).abs() < 1e-4 {
            self.gain = target;
        }
    }
}

/// Highest normalised autocorrelation over the pitch lags, and its lag.
fn pitch(w: &[f32; WIN]) -> (f32, usize) {
    let mean = w.iter().sum::<f32>() / WIN as f32;
    let mut s = [0.0f32; WIN];
    for (d, x) in s.iter_mut().zip(w) {
        *d = x - mean;
    }
    // Prefix sums of squares give the energy of both overlapping parts for every lag.
    let mut cum = [0.0f32; WIN + 1];
    for i in 0..WIN {
        cum[i + 1] = cum[i] + s[i] * s[i];
    }
    if cum[WIN] < 1e-12 {
        return (0.0, 0);
    }
    let mut best = (0.0, 0);
    for lag in MIN_LAG..=MAX_LAG {
        let n = WIN - lag;
        let dot: f32 = s[..n].iter().zip(&s[lag..]).map(|(a, b)| a * b).sum();
        let energy = (cum[n] * (cum[WIN] - cum[lag])).sqrt() + 1e-12;
        let c = dot / energy;
        if c > best.0 {
            best = (c, lag);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vowel-like sound: 150 Hz with a few harmonics.
    fn vowel(n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / FS;
                (1..=5)
                    .map(|h| (2.0 * std::f32::consts::PI * 150.0 * h as f32 * t).sin() / h as f32)
                    .sum::<f32>()
                    * amp
            })
            .collect()
    }

    /// Quiet background noise with short decaying "clicks" every 300 ms.
    fn typing(n: usize) -> Vec<f32> {
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        };
        (0..n)
            .map(|i| {
                let k = i % (FS as usize * 3 / 10);
                let click = if k < 240 {
                    0.3 * (-(k as f32) / 40.0).exp()
                } else {
                    0.0
                };
                rnd() * (0.001 + click)
            })
            .collect()
    }

    fn run(g: &mut SpeechGate, x: &[f32]) -> Vec<f32> {
        let mut y = x.to_vec();
        for hop in y.chunks_exact_mut(HOP) {
            g.analyze(hop);
            g.apply(hop, true);
        }
        y
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn mutes_clicks_between_words() {
        let mut g = SpeechGate::new();
        let x = typing(HOP * 300);
        let y = run(&mut g, &x);
        // After the first hops (the gate starts open), the clicks are gone.
        assert!(rms(&y[HOP * 50..]) < rms(&x[HOP * 50..]) * 0.01);
    }

    #[test]
    fn lets_voiced_speech_through_and_holds_after_it() {
        let mut g = SpeechGate::new();
        let mut x = typing(HOP * 100);
        x.extend(vowel(HOP * 50, 0.2));
        x.extend(vec![0.0; HOP * 50]);
        let y = run(&mut g, &x);
        let speech = HOP * 105..HOP * 150; // after the first two hops of the vowel
        assert!(rms(&y[speech.clone()]) > rms(&x[speech]) * 0.99);
        assert!(g.hold == 0, "gate must close again after the hold time");
        // Right after the vowel the gate is still open (hold), so a word ending is not cut.
        let mut g = SpeechGate::new();
        let mut x2 = typing(HOP * 100);
        x2.extend(vowel(HOP * 50, 0.2));
        x2.extend(typing(HOP * 10));
        let y2 = run(&mut g, &x2);
        let tail = HOP * 150..HOP * 160;
        assert!(rms(&y2[tail.clone()]) > rms(&x2[tail]) * 0.9);
    }

    #[test]
    fn settings_change_hold_and_depth() {
        let mut x = typing(HOP * 100);
        x.extend(vowel(HOP * 50, 0.2));
        x.extend(typing(HOP * 100));
        // A long hold keeps the gate open for the clicks right after the vowel.
        let mut g = SpeechGate::new();
        g.set_params(GateParams {
            hold_ms: 800,
            ..GateParams::default()
        });
        let y = run(&mut g, &x);
        let after = HOP * 155..HOP * 220;
        assert!(rms(&y[after.clone()]) > rms(&x[after.clone()]) * 0.9);
        // A shallow depth only lowers the pauses by 12 dB instead of muting them.
        let mut g = SpeechGate::new();
        g.set_params(GateParams {
            depth_db: -12.0,
            ..GateParams::default()
        });
        let y = run(&mut g, &x);
        let pause = HOP * 220..HOP * 250;
        let ratio = rms(&y[pause.clone()]) / rms(&x[pause]);
        assert!((ratio - 10f32.powf(-12.0 / 20.0)).abs() < 0.02, "{ratio}");
        // Out-of-range values are clamped.
        let p = GateParams {
            hold_ms: 99_999,
            depth_db: 5.0,
            ..GateParams::default()
        }
        .sanitized();
        assert_eq!(p.hold_ms, GateParams::MAX_HOLD_MS);
        assert_eq!(p.depth_db, -3.0);
    }

    #[test]
    fn inactive_gate_is_bit_exact() {
        let mut g = SpeechGate::new();
        let x = typing(HOP * 100);
        let mut y = x.clone();
        for hop in y.chunks_exact_mut(HOP) {
            g.analyze(hop);
            g.apply(hop, false);
        }
        assert_eq!(x, y);
    }
}
