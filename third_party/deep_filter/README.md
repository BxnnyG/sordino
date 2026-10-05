# deep_filter (vendored libDF)

This is the Rust core of [DeepFilterNet](https://github.com/Rikorose/DeepFilterNet) by Hendrik
Schröter, copied from commit `d375b2d8309e0935d165700c91da9de862a99c31`, licensed under
MIT or Apache-2.0 (see `LICENSE-MIT` and `LICENSE-APACHE`). The model file
`models/DeepFilterNet3_onnx.tar.gz` is DeepFilterNet3 from the same repository.

Why vendored: upstream's `libDF` is only available as a git dependency and pins `tract 0.21.4`,
which has a known vulnerability (RUSTSEC-2026-0217, fixed in 0.21.16). Vendoring lets Sordino build
offline, use a patched tract and avoid a moving git dependency.

Changes against upstream (everything else is unmodified):

* `src/lib.rs`: removed the modules and tests Sordino does not use (dataset loading, C API, wasm,
  logging channel, resampling transforms).
* `src/tract.rs`: `symbol_table` renamed to `symbols` (tract >= 0.21.5 API), model path adjusted.
* `Cargo.toml`: reduced to the dependencies the remaining code needs.
