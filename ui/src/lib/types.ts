// Mirrors crates/sordino-core/src/ipc.rs, settings.rs and studio.rs.

export type Status = 'off' | 'starting' | 'running' | 'mic_missing' | 'no_pipewire' | 'error';
export type Strength = 'light' | 'medium' | 'high' | 'max';
export type Preset = 'off' | 'natural' | 'clear' | 'warm' | 'custom';
export type ProfileKind = 'voice' | 'studio' | 'stereo' | 'headset' | 'music' | 'off' | 'other';
export type DeviceKind =
  | 'usb'
  | 'builtin'
  | 'bluetooth'
  | 'headset'
  | 'headphones'
  | 'speaker'
  | 'hdmi'
  | 'webcam'
  | 'loopback'
  | 'other';

export interface StudioParams {
  lowcut_hz: number;
  warmth_db: number;
  presence_db: number;
  air_db: number;
  deess: number;
  compression: number;
  gate: number;
  limiter: boolean;
}

export type Mode = 'call' | 'streaming' | 'recording';
export type RoomSize = 'small' | 'medium' | 'large';

export interface NoiseSettings {
  enabled: boolean;
  strength: Strength;
  pause_mute: boolean;
  auto_level: boolean;
  dereverb: RoomSize | null;
}

export interface Settings {
  enabled: boolean;
  mic: string | null;
  set_default: boolean;
  run_in_background: boolean;
  show_all_devices: boolean;
  auto_eq: boolean;
  muted: boolean;
  mic_level: { volume: number | null; avoid_clipping: boolean };
  noise: NoiseSettings;
  output_level: { volume: number | null };
  notify_muted_talk: boolean;
  mode: Mode;
  modes: Record<Mode, { noise: NoiseSettings; studio: { preset: Preset; custom: StudioParams }; auto_eq: boolean }>;
  onboarded: boolean;
  update_check: boolean;
  echo: { enabled: boolean };
  studio: { preset: Preset; custom: StudioParams };
  speaker: { enabled: boolean; strength: Strength; output: string | null; level_voices: boolean };
}

export interface ProfileInfo {
  index: number;
  name: string;
  description: string;
  kind: ProfileKind;
  available: boolean;
  priority: number;
}

export interface Device {
  id: string;
  node_id: number;
  name: string;
  kind: DeviceKind;
  card: number | null;
  profile: ProfileInfo | null;
}

export interface ProfileHint {
  card: number;
  device_id: string;
  device_name: string;
  current: ProfileInfo;
  suggested: ProfileInfo;
}

export interface SordinoState {
  version: string;
  status: Status;
  error: string | null;
  settings: Settings;
  devices: Device[];
  hidden_devices: Device[];
  active_mic: string | null;
  sinks: Device[];
  default_source: string | null;
  default_sink: string | null;
  speaker_active: boolean;
  auto_eq_gains: number[];
  speaker_output: string | null;
  diag: Diag;
  profile_hint: ProfileHint | null;
  latency_ms: number | null;
  default_is_sordino: boolean;
  monitoring: boolean;
  ab_original: boolean;
  echo_available: boolean;
  presets: Record<string, StudioParams>;
  mic_volume: number | null;
  output_volume: number | null;
  talking_while_muted: boolean;
  output_muted: boolean;
  panic: boolean;
}

export interface Diag {
  clipped_hops?: number;
  out_underruns: number;
  out_skipped: number;
  in_dropped: number;
  model_errors: number;
  quantum: number;
  out_callbacks: number;
  dsp_priority: number;
  overload_hops: number;
  overload_events: number;
}

export interface AutostartStatus {
  app: boolean;
  daemon: boolean;
  flatpak: boolean;
}

export interface Levels {
  input_db: number;
  output_db: number;
}

/** Deep partial used for settings patches. */
export type Patch<T> = { [K in keyof T]?: T[K] extends object ? Patch<T[K]> : T[K] };

export interface UpdateAvailable {
  version: string;
  tag: string;
  notes: string;
  url: string;
  can_install: boolean;
  reason: string | null;
}

export interface UpdateStatus {
  current: string;
  available: UpdateAvailable | null;
  error: string | null;
  checked_at: number | null;
}
