# Changelog

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
