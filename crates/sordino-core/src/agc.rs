//! Automatic level ("auto volume"): keeps speech at a steady loudness.
//!
//! For the microphone it makes you equally loud for the others whether you lean back or lean in;
//! for "Sordino Speaker" it evens out quiet and loud colleagues.
//!
//! Rules that keep it inaudible:
//! * It only *learns* on hops the noise model rates as speech, so pauses and noise never pull the
//!   gain up. In between it holds its gain.
//! * The speech level is a ~1.5 s average in dB (a single shout does not dominate it), and the
//!   gain moves at most 3 dB/s up and 6 dB/s down, so syllables are not flattened and nothing
//!   pumps.
//! * The gain is limited to +-12 dB and capped per hop so that peaks stay below -1 dBFS.

use crate::HOP;

/// Target speech level: average of the speech hops' RMS in dB (dBFS). Ordinary speech averaged
/// this way reads about 3 dB below its power average, so this is roughly -20 dBFS RMS speech.
pub const TARGET_DB: f32 = -23.0;
const MAX_BOOST_DB: f32 = 12.0;
const MAX_CUT_DB: f32 = -12.0;
/// Gain change per hop (10 ms): 3 dB/s up, 6 dB/s down.
const UP_DB_PER_HOP: f32 = 0.03;
const DOWN_DB_PER_HOP: f32 = 0.06;
/// Speech level average: about 1.5 s of speech.
const LEVEL_ALPHA: f32 = 1.0 / 150.0;
/// Peaks after the gain stay below this (-1 dBFS).
const CEILING: f32 = 0.891;
/// Hops quieter than this are never treated as speech (-55 dBFS).
const MIN_SPEECH_DB: f32 = -55.0;

pub struct Agc {
    /// Average speech level in dB, `None` until the first speech hop.
    level: Option<f32>,
    gain_db: f32,
    /// Linear gain at the end of the last hop (start of the per-sample ramp).
    last_gain: f32,
}

impl Default for Agc {
    fn default() -> Self {
        Self::new()
    }
}

impl Agc {
    pub fn new() -> Self {
        Agc {
            level: None,
            gain_db: 0.0,
            last_gain: 1.0,
        }
    }

    /// Stage switched off: forget the measurement and ramp back to unity gain within one hop.
    pub fn bypass(&mut self, hop: &mut [f32]) {
        self.level = None;
        self.gain_db = 0.0;
        if self.last_gain != 1.0 {
            self.process(hop, false);
            if (self.last_gain - 1.0).abs() < 1e-6 {
                self.last_gain = 1.0;
            }
        }
    }

    /// Current gain in dB.
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// Process one hop in place. `speech` says whether the hop is speech (from the noise model).
    pub fn process(&mut self, hop: &mut [f32], speech: bool) {
        debug_assert_eq!(hop.len(), HOP);
        let power = hop.iter().map(|x| x * x).sum::<f32>() / hop.len() as f32;
        let hop_db = 10.0 * (power + 1e-12).log10();
        if speech && hop_db > MIN_SPEECH_DB {
            let level_db = self.level.get_or_insert(hop_db);
            *level_db += LEVEL_ALPHA * (hop_db - *level_db);
            let level_db = *level_db;
            let want = (TARGET_DB - level_db).clamp(MAX_CUT_DB, MAX_BOOST_DB);
            self.gain_db += (want - self.gain_db).clamp(-DOWN_DB_PER_HOP, UP_DB_PER_HOP);
        }
        let peak = hop.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let mut target = 10f32.powf(self.gain_db / 20.0);
        if peak * target > CEILING {
            target = CEILING / peak;
        }
        // Ramp from the last gain, but never above what keeps this hop's peak under the ceiling.
        let cap = CEILING / peak.max(1e-9);
        let start = self.last_gain;
        let n = hop.len() as f32;
        for (i, x) in hop.iter_mut().enumerate() {
            let g = start + (target - start) * (i as f32 + 1.0) / n;
            *x *= g.min(cap);
        }
        self.last_gain = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48000.0).sin())
            .collect()
    }

    fn rms_db(x: &[f32]) -> f32 {
        10.0 * (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).log10()
    }

    fn run(agc: &mut Agc, x: &[f32], speech: bool) -> Vec<f32> {
        let mut y = x.to_vec();
        for hop in y.chunks_exact_mut(HOP) {
            agc.process(hop, speech);
        }
        y
    }

    #[test]
    fn quiet_speech_is_raised_towards_the_target() {
        let mut agc = Agc::new();
        let x = tone(HOP * 1000, 0.02); // about -37 dBFS: far too quiet
        let y = run(&mut agc, &x, true);
        let tail = &y[HOP * 800..];
        assert!(
            (agc.gain_db() - MAX_BOOST_DB).abs() < 0.1,
            "{}",
            agc.gain_db()
        );
        assert!(rms_db(tail) > rms_db(&x[HOP * 800..]) + 11.0);
    }

    #[test]
    fn loud_speech_is_lowered_and_peaks_stay_below_the_ceiling() {
        let mut agc = Agc::new();
        let x = tone(HOP * 600, 0.7); // about -6 dBFS
        let y = run(&mut agc, &x, true);
        assert!(agc.gain_db() < -10.0, "{}", agc.gain_db());
        assert!(y.iter().all(|v| v.abs() <= CEILING + 1e-4));
        // A sudden loud burst after a quiet passage is capped at once.
        let mut agc = Agc::new();
        run(&mut agc, &tone(HOP * 1000, 0.02), true);
        let burst = run(&mut agc, &tone(HOP * 5, 0.9), true);
        assert!(burst.iter().all(|v| v.abs() <= CEILING + 1e-4));
    }

    #[test]
    fn noise_never_moves_the_gain() {
        let mut agc = Agc::new();
        run(&mut agc, &tone(HOP * 500, 0.01), false);
        assert_eq!(agc.gain_db(), 0.0);
    }

    #[test]
    fn gain_moves_slowly() {
        let mut agc = Agc::new();
        run(&mut agc, &tone(HOP * 100, 0.02), true); // one second of quiet speech
        assert!(agc.gain_db() <= 3.0 + 1e-3, "{}", agc.gain_db());
    }
}
