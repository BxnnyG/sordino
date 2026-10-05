# Contributing

Thanks for helping. A few notes that save everybody time.

## Layout

| Path | What |
|---|---|
| `crates/sordino-core` | Settings, DSP chain, DeepFilterNet wrapper, echo canceller. No PipeWire, no D-Bus. |
| `crates/sordinod` | The daemon: PipeWire client, DSP worker thread, D-Bus API. |
| `crates/sordinoctl` | Command line client. |
| `ui/` | Svelte frontend, `ui/src-tauri` is the Tauri shell. |
| `dist/` | Desktop file, D-Bus activation file, systemd user unit. |
| `packaging/` | PKGBUILD (`aur/`), deb, rpm, Flatpak, tarball builders. |
| `third_party/deep_filter` | Vendored DeepFilterNet core (MIT/Apache-2.0), see its README for the diff to upstream. |
| `tools/` | Quality benchmark (`eval_quality.py`), recording helper and analysis, dev container. |
| `tests/e2e` | End-to-end tests against a private PipeWire. |

## Build and test

```sh
cd ui && npm ci && npm run build && cd ..   # the Tauri crate embeds ui/dist
cargo build --release
cargo test --release --workspace
```

Use `--release` for anything that loads the model: DeepFilterNet's graph optimiser trips over
debug assertions. `cargo build --profile fast` is a quicker optimised profile (no LTO) for iteration.

## Rules that keep Sordino from breaking your audio

* Sordino never writes PipeWire configuration. Everything is a normal PipeWire client object.
* No allocation, locking or model inference in PipeWire's real-time callbacks. They only copy
  samples into ring buffers; the DSP runs on its own thread.
* Every error that can happen in the chain must end up as a readable message in the UI.
* `pipewire-rs` requires all PipeWire calls on the main thread.
* Callbacks never re-enter the engine synchronously; they enqueue a command.

## Measuring quality

Changes to the audio path must not make it worse. Before and after a change run

```sh
cargo build --release -p sordino-core --example process_file
python3 tools/eval_quality.py --args "--noise high --studio off" --args "--noise high --studio natural"
```

It reports SI-SDR, STOI and PESQ on real speech mixed with noise at 20/10/0 dB SNR. For reference
(0.1.0, `--noise high --studio off`): 19.9 dB / 0.976 / 3.10.

## End-to-end tests

`tests/e2e/run.sh` starts its own PipeWire and WirePlumber in a throw-away session and checks
continuity, hotplug, default-device restore, the monitor safety net and `kill -9` behaviour. It
does not touch your running audio. In a container: `tools/dcargo.sh tests/e2e/run.sh`.

## Before opening a PR

`cargo fmt --all`, `cargo clippy --release --workspace --all-targets`, `cargo test --release --workspace`,
`cargo deny check` and `cd ui && npm run check`.

## License of contributions

By contributing you agree that your work is licensed under GPL-3.0-or-later with the attribution terms in [NOTICE](NOTICE).

## Releasing

1. Bump `version` in `Cargo.toml` (workspace), `ui/src-tauri/tauri.conf.json` and `packaging/aur/PKGBUILD`,
   add a section to `CHANGELOG.md`.
2. Commit, then `git tag vX.Y.Z && git push --tags`.
3. The `Release` workflow builds the deb, rpm, Arch package, Flatpak bundle and tarball, writes
   `SHA256SUMS` and publishes the GitHub release (marked pre-release while the version is 0.x).
   "Run workflow" on the Release workflow does a dry run without publishing.
