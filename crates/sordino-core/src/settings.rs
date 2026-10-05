//! Persisted user settings (`~/.config/sordino/config.toml`) and runtime state.
//!
//! Unknown keys are ignored and missing ones fall back to defaults, so old and new versions
//! can read each other's files. A broken file is moved aside instead of crashing the daemon.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::denoise::Strength;
use crate::dereverb::RoomSize;
use crate::pipeline::PipelineParams;
use crate::studio::{Preset, StudioParams};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct NoiseSettings {
    pub enabled: bool,
    pub strength: Strength,
    /// Mute between words, so key clicks in pauses are not heard (see `speech_gate`).
    pub pause_mute: bool,
    /// Keep your voice at a steady loudness (see `agc`).
    pub auto_level: bool,
    /// Reduce room echo for a room of this size (see `dereverb`); `None` = off.
    pub dereverb: Option<RoomSize>,
}

impl Default for NoiseSettings {
    fn default() -> Self {
        NoiseSettings {
            enabled: true,
            strength: Strength::High,
            pause_mute: true,
            auto_level: true,
            dereverb: None,
        }
    }
}

/// Cleaning what you *hear*: apps play into the virtual output "Sordino Speaker", Sordino removes
/// noise from the other people's voices and forwards the result to the real headphones/speakers.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct SpeakerSettings {
    pub enabled: bool,
    pub strength: Strength,
    /// `node.name` of the real output device; `None` follows the system default output.
    pub output: Option<String>,
    /// Even out quiet and loud voices.
    pub level_voices: bool,
}

impl Default for SpeakerSettings {
    fn default() -> Self {
        SpeakerSettings {
            enabled: false,
            strength: Strength::Medium,
            output: None,
            level_voices: true,
        }
    }
}

impl SpeakerSettings {
    pub fn pipeline_params(&self) -> PipelineParams {
        PipelineParams {
            echo: false,
            noise: true,
            strength: self.strength,
            // Correcting a microphone makes no sense for other people's audio.
            auto_eq: false,
            studio: None,
            // Other people's apps already decide when they talk.
            pause_mute: false,
            mute: false,
            agc: self.level_voices,
            dereverb: None,
        }
    }
}

/// Input level of the physical microphone, applied whenever it appears.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct MicLevelSettings {
    /// 0..1 on the same scale as `wpctl set-volume` and the desktop's sound settings;
    /// `None` leaves the level alone.
    pub volume: Option<f32>,
    /// Lower the level a little whenever the microphone clips.
    pub avoid_clipping: bool,
}

impl Default for MicLevelSettings {
    fn default() -> Self {
        MicLevelSettings {
            volume: None,
            avoid_clipping: true,
        }
    }
}

impl MicLevelSettings {
    /// Lowest level the clipping guard goes down to on its own.
    pub const GUARD_FLOOR: f32 = 0.4;

    pub fn sanitized_volume(&self) -> Option<f32> {
        self.volume
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0))
    }
}

/// A situation the user switches between. Each mode remembers its own sound settings.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Calls and meetings: strong cleaning, silence between words, steady level.
    #[default]
    Call,
    /// Streaming: continuous voice (no pause muting), a little more presence.
    Streaming,
    /// Recording: lighter cleaning and natural dynamics (no automatic level).
    Recording,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Call, Mode::Streaming, Mode::Recording];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Call => "call",
            Mode::Streaming => "streaming",
            Mode::Recording => "recording",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// The sound settings a mode remembers.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct ModeValues {
    pub noise: NoiseSettings,
    pub studio: StudioSettings,
    pub auto_eq: bool,
}

impl Default for ModeValues {
    fn default() -> Self {
        Mode::Call.defaults()
    }
}

impl Mode {
    /// What a mode starts with before the user changes anything.
    pub fn defaults(self) -> ModeValues {
        let studio = |preset| StudioSettings {
            preset,
            custom: StudioParams::default(),
        };
        match self {
            Mode::Call => ModeValues {
                noise: NoiseSettings::default(),
                studio: studio(Preset::Natural),
                auto_eq: true,
            },
            Mode::Streaming => ModeValues {
                noise: NoiseSettings {
                    pause_mute: false,
                    ..NoiseSettings::default()
                },
                studio: studio(Preset::Clear),
                auto_eq: true,
            },
            Mode::Recording => ModeValues {
                noise: NoiseSettings {
                    strength: Strength::Medium,
                    pause_mute: false,
                    auto_level: false,
                    ..NoiseSettings::default()
                },
                studio: studio(Preset::Natural),
                auto_eq: true,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct ModeStore {
    pub call: ModeValues,
    pub streaming: ModeValues,
    pub recording: ModeValues,
}

impl Default for ModeStore {
    fn default() -> Self {
        ModeStore {
            call: Mode::Call.defaults(),
            streaming: Mode::Streaming.defaults(),
            recording: Mode::Recording.defaults(),
        }
    }
}

impl ModeStore {
    pub fn get(&self, m: Mode) -> &ModeValues {
        match m {
            Mode::Call => &self.call,
            Mode::Streaming => &self.streaming,
            Mode::Recording => &self.recording,
        }
    }

    fn get_mut(&mut self, m: Mode) -> &mut ModeValues {
        match m {
            Mode::Call => &mut self.call,
            Mode::Streaming => &mut self.streaming,
            Mode::Recording => &mut self.recording,
        }
    }
}

/// Volume of the real output (headphones/speakers), applied whenever it appears.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct OutputLevelSettings {
    /// 0..1 on the `wpctl` scale; `None` leaves the volume alone.
    pub volume: Option<f32>,
}

impl OutputLevelSettings {
    pub fn sanitized_volume(&self) -> Option<f32> {
        self.volume
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0))
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct EchoSettings {
    pub enabled: bool,
    /// `node.name` of the sink (or source) whose signal is the echo reference. `None` uses the
    /// default output device, which is right unless you play through something unusual.
    pub reference: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct StudioSettings {
    pub preset: Preset,
    /// Values used when `preset == Custom`; the UI seeds them from the last preset.
    pub custom: StudioParams,
}

impl StudioSettings {
    /// Effective parameters, `None` when studio sound is off.
    pub fn effective(&self) -> Option<StudioParams> {
        match self.preset {
            Preset::Off => None,
            Preset::Custom => Some(self.custom.sanitized()),
            p => p.params(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    /// Sordino Mic exists and processes audio.
    pub enabled: bool,
    /// `node.name` of the physical microphone; `None` follows the system default.
    pub mic: Option<String>,
    /// Make Sordino Mic the system default microphone while running.
    pub set_default: bool,
    /// Keep running in the tray when the window is closed.
    pub run_in_background: bool,
    pub show_all_devices: bool,
    /// Automatically correct the microphone's tonal balance (see `autoeq`).
    pub auto_eq: bool,
    /// Sordino Mic sends silence (mute button / panic). Kept across restarts on purpose: a crash
    /// must never turn a muted microphone back on.
    pub muted: bool,
    pub mic_level: MicLevelSettings,
    pub output_level: OutputLevelSettings,
    /// Show a desktop notification when you talk while muted.
    pub notify_muted_talk: bool,
    /// Current situation; sound changes are remembered for it.
    pub mode: Mode,
    pub modes: ModeStore,
    /// The setup assistant has been completed (or skipped).
    pub onboarded: bool,
    /// The app asks GitHub once a day whether a new version exists (only that request, no data
    /// about the user or the system besides what any web request carries).
    pub update_check: bool,
    pub noise: NoiseSettings,
    pub echo: EchoSettings,
    pub studio: StudioSettings,
    pub speaker: SpeakerSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            mic: None,
            set_default: false,
            run_in_background: true,
            show_all_devices: false,
            auto_eq: true,
            muted: false,
            mic_level: MicLevelSettings::default(),
            output_level: OutputLevelSettings::default(),
            notify_muted_talk: true,
            mode: Mode::Call,
            modes: ModeStore::default(),
            onboarded: false,
            update_check: true,
            noise: NoiseSettings::default(),
            echo: EchoSettings::default(),
            studio: StudioSettings::default(),
            speaker: SpeakerSettings::default(),
        }
    }
}

impl Settings {
    pub fn pipeline_params(&self) -> PipelineParams {
        PipelineParams {
            echo: self.echo.enabled,
            noise: self.noise.enabled,
            strength: self.noise.strength,
            auto_eq: self.auto_eq,
            studio: self.studio.effective(),
            pause_mute: self.noise.pause_mute,
            mute: self.muted,
            agc: self.noise.auto_level,
            dereverb: self.noise.dereverb.filter(|_| self.noise.enabled),
        }
    }

    /// Merge a JSON patch (RFC 7386 style: objects merge, everything else replaces).
    pub fn patched(&self, patch: &serde_json::Value) -> Result<Settings> {
        let mut base = serde_json::to_value(self)?;
        merge(&mut base, patch);
        let mut s: Settings = serde_json::from_value(base).context("invalid settings patch")?;
        s.studio.custom = s.studio.custom.sanitized();
        if s.mode != self.mode {
            // Switching modes brings back what that mode remembers.
            let v = s.modes.get(s.mode).clone();
            s.noise = v.noise;
            s.studio = v.studio;
            s.auto_eq = v.auto_eq;
        } else {
            // Any other change is remembered for the current mode.
            let m = s.mode;
            *s.modes.get_mut(m) = s.mode_values();
        }
        Ok(s)
    }

    /// The current sound settings as a mode remembers them.
    pub fn mode_values(&self) -> ModeValues {
        ModeValues {
            noise: self.noise.clone(),
            studio: self.studio.clone(),
            auto_eq: self.auto_eq,
        }
    }
}

fn merge(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                merge(b.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (b, p) => *b = p.clone(),
    }
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() && PathBuf::from(&v).is_absolute() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback),
    }
}

pub fn config_path() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
        .join("sordino")
        .join("config.toml")
}

pub fn state_path() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
        .join("sordino")
        .join("state.toml")
}

/// Write atomically: a crash mid-write must never leave a half-written config behind.
fn write_atomic(path: &PathBuf, contents: &str) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

impl Settings {
    pub fn load() -> Settings {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &PathBuf) -> Settings {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return Settings::default(),
        };
        match toml::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.studio.custom = s.studio.custom.sanitized();
                s
            }
            Err(e) => {
                log::warn!(
                    "config {} is invalid ({e}); moving it aside and using defaults",
                    path.display()
                );
                let _ = fs::rename(path, path.with_extension("toml.broken"));
                Settings::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &PathBuf) -> Result<()> {
        write_atomic(path, &toml::to_string_pretty(self)?)
    }
}

/// Things the daemon must remember across restarts but that are not user preferences.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct RuntimeState {
    /// Value of `default.configured.audio.source` before Sordino overrode it.
    /// `Some("")` means "there was none". `None` means Sordino did not touch the default.
    pub previous_default_source: Option<String>,
}

impl RuntimeState {
    pub fn load() -> RuntimeState {
        fs::read_to_string(state_path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        write_atomic(&state_path(), &toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sordino-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.join("config.toml")
    }

    #[test]
    fn roundtrip() {
        let p = tmp("roundtrip");
        let s = Settings {
            mic: Some("alsa_input.usb-x".into()),
            noise: NoiseSettings {
                strength: Strength::Max,
                ..Default::default()
            },
            studio: StudioSettings {
                preset: Preset::Clear,
                ..Default::default()
            },
            ..Default::default()
        };
        s.save_to(&p).unwrap();
        assert_eq!(Settings::load_from(&p), s);
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(Settings::load_from(&tmp("missing")), Settings::default());
    }

    #[test]
    fn partial_and_unknown_keys_are_tolerated() {
        let p = tmp("partial");
        fs::write(&p, "future_key = 1\n[noise]\nstrength = \"light\"\n").unwrap();
        let s = Settings::load_from(&p);
        assert_eq!(s.noise.strength, Strength::Light);
        assert!(s.noise.enabled);
        assert!(s.enabled);
    }

    #[test]
    fn broken_file_is_moved_aside() {
        let p = tmp("broken");
        fs::write(&p, "this is [not toml").unwrap();
        assert_eq!(Settings::load_from(&p), Settings::default());
        assert!(!p.exists());
        assert!(p.with_extension("toml.broken").exists());
    }

    #[test]
    fn hand_edited_garbage_values_are_sanitized() {
        let p = tmp("garbage");
        fs::write(
            &p,
            "[studio.custom]\nwarmth_db = 99.0\ncompression = -4.0\n",
        )
        .unwrap();
        let s = Settings::load_from(&p);
        assert_eq!(s.studio.custom.warmth_db, 6.0);
        assert_eq!(s.studio.custom.compression, 0.0);
    }

    #[test]
    fn patch_merges_nested_objects() {
        let s = Settings::default();
        let n = s
            .patched(&serde_json::json!({"noise": {"strength": "max"}, "set_default": true}))
            .unwrap();
        assert_eq!(n.noise.strength, Strength::Max);
        assert!(n.noise.enabled, "sibling keys must survive");
        assert!(n.set_default);
        assert!(s
            .patched(&serde_json::json!({"noise": {"strength": "bogus"}}))
            .is_err());
    }

    #[test]
    fn speaker_defaults_off_and_uses_only_noise_suppression() {
        let s = Settings::default();
        assert!(!s.speaker.enabled);
        let p = s.speaker.pipeline_params();
        assert!(p.noise && !p.echo && p.studio.is_none());
        assert!(!p.pause_mute);
    }

    #[test]
    fn modes_remember_their_own_settings() {
        let s = Settings::default();
        assert_eq!(s.mode, Mode::Call);
        // Change something in "call", then switch to "recording": its defaults come in.
        let s = s
            .patched(&serde_json::json!({"noise": {"strength": "max"}}))
            .unwrap();
        assert_eq!(s.modes.call.noise.strength, Strength::Max);
        let r = s
            .patched(&serde_json::json!({"mode": "recording"}))
            .unwrap();
        assert_eq!(r.noise.strength, Strength::Medium);
        assert!(!r.noise.auto_level && !r.noise.pause_mute);
        // A change in "recording" stays in "recording"...
        let r = r
            .patched(&serde_json::json!({"studio": {"preset": "warm"}}))
            .unwrap();
        assert_eq!(r.modes.recording.studio.preset, Preset::Warm);
        // ...and switching back restores "call" exactly as it was left.
        let c = r.patched(&serde_json::json!({"mode": "call"})).unwrap();
        assert_eq!(c.noise.strength, Strength::Max);
        assert_eq!(c.studio.preset, Preset::Natural);
        assert!(c.noise.pause_mute);
        assert_eq!(Mode::parse("streaming"), Some(Mode::Streaming));
    }

    #[test]
    fn mute_and_mic_level_defaults() {
        let s = Settings::default();
        assert!(!s.muted && !s.pipeline_params().mute);
        assert_eq!(s.mic_level.volume, None);
        assert!(s.mic_level.avoid_clipping);
        let n = s
            .patched(&serde_json::json!({"muted": true, "mic_level": {"volume": 1.7}}))
            .unwrap();
        assert!(n.pipeline_params().mute);
        assert_eq!(n.mic_level.sanitized_volume(), Some(1.0));
    }

    #[test]
    fn pause_mute_is_on_by_default_and_old_files_get_it() {
        assert!(Settings::default().pipeline_params().pause_mute);
        let old: Settings = toml::from_str("[noise]\nstrength = \"medium\"\n").unwrap();
        assert!(old.noise.pause_mute);
    }

    #[test]
    fn studio_effective() {
        let mut s = StudioSettings::default();
        assert_eq!(s.effective(), Preset::Natural.params());
        s.preset = Preset::Off;
        assert_eq!(s.effective(), None);
        s.preset = Preset::Custom;
        s.custom.warmth_db = 2.0;
        assert_eq!(s.effective().unwrap().warmth_db, 2.0);
    }
}
