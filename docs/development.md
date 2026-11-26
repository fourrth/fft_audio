# Development

## Prerequisites

* Rust/Cargo 1.100+
* Node.js 26+ / npm
* Tauri 2 Linux: `webkit2gtk-4.1`, `gtk+-3.0`, `librsvg-2.0`, `pkg-config`
* Python 3.10+, NumPy, and Python development headers for `tauri_python`/`python` builds

## Frontend

```sh
npm --prefix web install
npm --prefix web run build     # writes web/dist
```

For feature builds that serve the UI (`http`, `tauri`, `tauri_python`), the product build script rebuilds stale
`web/dist` automatically when `web/node_modules` exists. It does not run `npm install`. Use
`FFT_SKIP_WEB_BUILD=1` for Rust-only work.

## Product modes

```sh
# Default GUI: Tauri + embedded Python
cargo run -p fft_desktop

# GUI without scripting
cargo run -p fft_desktop --no-default-features --features tauri

# No GUI, CLI composition (inert without --serve/scripts)
cargo run -p fft_desktop -- --headless

# Headless WebUI
cargo run -p fft_desktop --no-default-features --features http -- \
  --serve --port 8765 --bind 127.0.0.1 --dist web/dist

# CLI Python, no listener
cargo run -p fft_desktop --no-default-features --features python -- \
  --python-script examples/harmonics_chirp_example.py --workers 1

# WebUI plus CLI Python
cargo run -p fft_desktop --no-default-features --features http,python -- \
  --serve --port 8765 --dist web/dist \
  --python-script examples/three_tones_example.py
```

`--serve` requires `http`; `--python-script` requires `python`. `--bind`, `--port`, and `--dist` require
`--serve`. `--headless` only controls GUI launch.

`--bind` defaults to `127.0.0.1:8765` and accepts any local IP or `IP:PORT`, including `0.0.0.0`. The WebUI has no
authentication, so non-loopback binds are appropriate only on trusted networks.

## CLI flags

```text
--workers <N>              runtime worker threads
--config-dir <DIR>         config/settings/session/themes/uploads directory
--headless                 do not launch the GUI
--fresh                    reset settings.toml, session.json, and themes/*.toml
--serve                    serve the WebUI until Ctrl-C
--bind <ADDR>              IP or IP:PORT to bind (with --serve)
--port <PORT>              WebUI port (with --serve)
--dist <DIR>               frontend build directory (with --serve)
--python-script <PATH>     local Python script (repeatable; requires python)
--exit-after-script        stop the WebUI after the last script
--python-delay <SECONDS>   wait before the first script
--python-wait              wait for Enter before the first script
--help, -h                 usage
--version, -V              version
```

`--fresh` clears only app config files. It never removes directories or symlinks and never touches uploads,
scripting examples, or unrelated files. It uses the run's resolved config directory; an explicit `--config-dir` is
not required.

## Tests and feature matrix

Use `scripts/dev.py` instead of invoking `cargo test`, `npm test`, or Playwright directly. It runs the Rust
feature matrix, frontend checks, and browser tests, and prints a grouped pass/fail summary. `--list` prints every
suite with its exact underlying command.

```sh
python3 scripts/dev.py                          # all areas
python3 scripts/dev.py --fast                   # Rust + frontend, no e2e/wasm
python3 scripts/dev.py rust                     # full Rust feature matrix
python3 scripts/dev.py rust/fmt rust/check rust/core
python3 scripts/dev.py frontend                 # typecheck + unit + build
python3 scripts/dev.py frontend e2e             # frontend + Playwright
python3 scripts/dev.py rust/wasm wasm           # fft_wasm tests + wasm32 check
python3 scripts/dev.py --skip e2e
python3 scripts/dev.py -v <targets>             # full command output
python3 scripts/dev.py --list
```

Areas are `rust`, `frontend`, `e2e`, and `wasm`; individual suite names such as `rust/core` or `frontend/unit`
are also accepted. `--fast`, `--skip`, `--fail-fast`, and `--no-color` are available. The runner executes suites
sequentially because Cargo feature builds share `target/`, builds `web/dist` automatically before Playwright, and
skips `wasm/check` in a default run when the `wasm32-unknown-unknown` target is not installed. Frontend and E2E
suites require `npm --prefix web install`; the runner does not install dependencies.

No test launches the GUI. Tauri command/event tests run without creating a window. The Playwright web server runs
`fft_desktop --no-default-features --features http --serve` against an isolated config directory. It uses
`web/dist` and resets settings/session/displays between tests.

Static browser/WASM build (optional):

```sh
python3 scripts/dev.py rust/wasm wasm
./build-web-static.sh
```

Serve `web/dist/` with any static web server. See [Static browser build](static-browser.md).

## Benchmarks

`fft_benchmarks` uses Criterion and benchmarks only the DSP internals, not adapters or transport.

```sh
# Rust FFT, spectrogram analysis, and render benchmarks
cargo bench -p fft_benchmarks

# Include direct NumPy comparisons
cargo bench -p fft_benchmarks --features python
```

The comparison uses the same deterministic samples, frame sizes (1024/4096/16384), 50% hop, no window, no filters,
and the same dB scaling on both sides. Rust measures `fft_core`/`realfft`; Python measures direct NumPy
`np.fft.rfft` code in-process. The Python path deliberately bypasses the app/runtime adapters so the benchmark
compares compute, not lease/transport overhead. Python `--features python` requires Python development headers and
NumPy. Criterion reports are written under `target/criterion/`.

## Shipping

`fft_shipping` reads `shipping.toml` or takes an ad-hoc package spec:

```sh
cargo run -p fft_shipping -- --list
cargo run -p fft_shipping                              # all jobs
cargo run -p fft_shipping -- --job headless
cargo run -p fft_shipping -- --dry-run
cargo run -p fft_shipping -- --command check --no-fetch
cargo run -p fft_shipping -- \
  --package fft_desktop --no-default-features --features http,python \
  --bin fft_desktop --name headless-python --release
```

Defaults: release profile, `target/shipping`, output `dist/`, `cargo fetch` on. Built binaries are copied to
`dist/<job>/<binary>`. Cargo artifacts live under `target/shipping/<profile>/`.

Useful overrides: `--debug`, `--target-dir`, `--out-dir`, `--no-fetch`, `--no-copy`, `--cargo-arg <ARG>`,
`--keep-going`, and `--workspace-root`. Binaries are copied only for `cargo build` jobs.

## Environment variables

| Variable | Purpose |
|---|---|
| `FFT_SKIP_WEB_BUILD=1` | Do not build `web/dist` from Cargo. |
| `FFT_AUDIO_CONFIG_DIR` | Default CLI config directory. |
| `FFT_AUDIO_FORCE_X11=1` | Force X11 (`GDK_BACKEND`/`WINIT_UNIX_BACKEND`) unless already set. |
| `__NV_DISABLE_EXPLICIT_SYNC` | NVIDIA Wayland workaround; set to `1` by default when unset. Set `0` to opt out. |
| `FFT_AUDIO_SCRIPT_WORKDIR` | Example script output directory. |
| `FFT_AUDIO_CHECK_LEVELS=1` | Enable the three-tone level assertion in the display example. |
| `FFT_AUDIO_IMAGE_PATH` | Optional PNG output path for the three-tone example. |

## Troubleshooting

* **Wayland crash with NVIDIA** — the Linux product sets `__NV_DISABLE_EXPLICIT_SYNC=1` when unset. Try
  `FFT_AUDIO_FORCE_X11=1 cargo run -p fft_desktop`.
* **GUI shows `Could not connect to 127.0.0.1`** — Tauri is in dev mode. Normal GUI features enable
  `custom-protocol`, which loads the embedded `web/dist`; rebuild with `cargo run -p fft_desktop` (or use the Tauri
  CLI's dev command when you intentionally want the Vite dev server).
* **Stale UI** — run `npm --prefix web run build`; for `http`/Tauri builds the build script also handles stale assets.
* **`--serve` rejected** — build with `--features http`.
* **`--python-script` rejected** — build with `--features python`.
* **`tauri` and `tauri_python` together** — intentionally a compile error; choose one GUI feature.
* **`--fresh` did nothing** — it logs when the config directory or known config files are absent. It never deletes
  directories, uploads, examples, or unrelated files.
