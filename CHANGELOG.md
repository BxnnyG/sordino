# Changelog

## 0.2.0

* **Modes: Call, Streaming, Recording.** Switch at the top of the window, with
  `sordinoctl mode …` or a desktop shortcut. Each mode remembers its own sound settings (noise
  suppression, silence between words, steady loudness, room echo, studio sound, mic correction).
* **Steady loudness** (automatic level): keeps your voice equally loud for the others. Learns
  only on what the noise model rates as speech, moves at most 3 dB/s up and 6 dB/s down, +-12 dB,
  peaks stay below -1 dBFS. On the test recording the speech level of the normal sections went
  from a 7.7 dB spread to 1.8 dB. On by default in Call and Streaming.
* **Even out voices** on Sordino Speaker: quiet and loud colleagues become equally loud.
* **Reduce room echo** (off by default): late-reverberation suppression for a small, medium or
  bare room, +10 ms latency while on. In synthetic rooms PESQ rose by 0.2-0.5 with unchanged
  intelligibility; in a dry room it costs a little (PESQ 4.53 -> 4.34 on "small"), hence off.
* **"You are talking, but you are muted"**: banner in the app and a desktop notification (at
  most once a minute, can be switched off).
* **Headphone volume** as a default, applied whenever the device appears
  (`sordinoctl output-level`).
* **Setup assistant**: microphone and automatic level check, mode, and what to switch off in
  Discord/Teams/Zoom. Can be started again from the settings.

## 0.1.3

* **Mute button and panic mute.** "Mute mic" silences Sordino Mic (it stays muted across
  restarts, so a crash never turns it back on). "Mute everything" also mutes your real
  headphones/speakers and undoes both with one click. In the app, the tray menu, the CLI
  (`sordinoctl mute`, `sordinoctl panic`) and as desktop actions, so you can bind them to a key
  in your desktop's shortcut settings (the desktop handles the key, Sordino reads no keys).
* **Input level.** Set your microphone's level in the app or with `sordinoctl mic-level 80`;
  Sordino applies it whenever the microphone appears (same scale as `wpctl set-volume`).
* **Clipping guard** (on by default): when the microphone distorts, Sordino lowers its level
  in small steps (never below 40 %). `sordinoctl clip-guard on|off`; clipped hops in `diag`.

## 0.1.2

* **Silence between words** (on by default). Sordino Mic is muted while you are not talking, so
  key clicks in pauses are not heard. It opens on voiced speech (a stable pitch) and holds for
  200 ms after the last word. No added latency: it analyses the microphone signal ahead of the
  noise model's own 30 ms delay. Measured on a recording with typing: about 60 % of the clicks
  between words are gone, voiced speech keeps 99.8 % of its energy. Typing *while* you talk and
  whispering are limits: the first is still heard, the second is muted too.
  `sordinoctl pause-mute on|off`.
* `sordinoctl echo on|off`; `sordinoctl status` shows echo suppression and pause mute.
* End-to-end test: the private D-Bus session no longer activates an installed Sordino.
* `process_file` can dump the noise model's per-hop speech estimate (`--lsnr-out`).

## 0.1.1

* **Automatic microphone correction** ("Fix my microphone's sound", on by default). Sordino
  learns the tonal balance of your speech and slowly corrects it towards a studio reference,
  bounded and only where your voice is above the noise. Measured on a cheap USB microphone:
  4-6 kHz from -9.5 dB to +1.3 dB, 6-8 kHz from -14.9 dB to -2.5 dB relative to a studio mic;
  on studio recordings it stays at about 0 dB. `sordinoctl autoeq on|off`.
* Tools: `speech_profile` example, auto EQ diagnostics in `process_file`.

## 0.1.0

First public preview.

* Virtual microphone **Sordino Mic** with DeepFilterNet 3 noise suppression (four strengths).
* **Sordino Speaker**: cleans incoming voices (the other people in a call) before they reach
  your headphones.
* Real-time priority for the processing thread, adaptive buffering and an overload fallback, so
  a busy machine no longer produces crackle.
* DeepFilterNet stage thresholds tuned on real speech (STOI 0.90 -> 0.98 against the library
  defaults); the deep-filtering stage no longer switches off for loud speech.
* Choose the system default microphone and output in the settings; autostart at login for the
  background service and the app.
* Studio sound presets (Natural, Clear, Warm) and an advanced view.
* Echo suppression for speakers (WebRTC AEC3), experimental.
* Profile fixer for microphones stuck on `pro-audio`, hotplug handling.
* Test mode with hear-yourself monitoring and a direct A/B button; monitoring expires on its own.
* Desktop app (de/en, light/dark), tray, autostart, D-Bus API, `sordinoctl` CLI.
* Packages: deb, rpm, Arch package, Flatpak bundle, tarball.
