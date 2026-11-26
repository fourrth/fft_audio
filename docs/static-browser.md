# Static browser build (WebAssembly)

`fft_audio` can be built as a static browser application: HTML, JavaScript, CSS,
and a WebAssembly module served from any static web host. There is no
application server, upload endpoint, or server-side file processing.

The normal product (`fft_desktop`) is unchanged. The browser build is an optional
third frontend path next to the Tauri desktop app and the headless HTTP WebUI.

## Build

Prerequisites:

- Rust target `wasm32-unknown-unknown` (`rustup target add wasm32-unknown-unknown`)
- `wasm-bindgen` CLI matching the `wasm-bindgen` crate in `Cargo.lock`
- `npm --prefix web install`

Build the WASM module and static bundle:

```sh
./build-web-static.sh
```

The script:

1. builds `fft_wasm` for `wasm32-unknown-unknown` in release mode,
2. runs `wasm-bindgen --target web` into `web/public/wasm/`,
3. runs `npm --prefix web run build:browser` into `web/dist/`.

Serve only `web/dist/`; for example:

```sh
python3 -m http.server 8080 --directory web/dist
```

Then open <http://127.0.0.1:8080>. The bundle uses relative asset URLs, so it
can also be served below a sub-path on a static host.

## What runs in the browser

- **`crates/fft_wasm`** is a thin `wasm-bindgen` wrapper around `fft_core`. It
  uses the lean `tables-text` feature only and has no DOM, filesystem, threading,
  or Python dependency.
- **`web/src/wasm/worker.ts`** loads the WASM module and performs decode,
  analysis, cuts, and PNG rendering off the UI thread.
- **`web/src/transport/browser.ts`** implements the same `Transport` contract as
  the desktop/headless paths. File bytes come from a browser file picker and are
  sent only to that worker.

Settings, session data, and user themes are stored in browser `localStorage`.
There are no host filesystem paths or upload requests.

## Scope and limits

- Single-threaded WASM. There is no `wasm-bindgen-rayon`, `SharedArrayBuffer`, or
  COOP/COEP requirement.
- Supported input:
  - audio formats decoded by `symphonia` and `symphonia-opus` (WAV, FLAC, MP3, Ogg/Vorbis, AAC, ALAC, Opus, ...),
  - text tables: CSV/TSV/JSON/NDJSON (the lean build disables Parquet and Arrow).
- Exports (spectrogram PNG and cut CSV) are browser downloads.
- Scripting is intentionally absent. `capabilities.scripting` is `null`, so the
  Scripting tile and script commands are unavailable.
- File-backed analyses are ephemeral: layout/settings persist, but after a page
  reload the user must choose the file again. The static build does not copy
  user files into browser storage.
- Analysis cancellation is not available while the worker is synchronously
  computing; there is no background job queue in this build.

## Verification

```sh
python3 scripts/dev.py rust/check rust/wasm frontend wasm
npm --prefix web run build:browser   # static bundle, not a test suite
```

`rust/check` covers the workspace compile, `rust/wasm` the host-side `fft_wasm` tests, `frontend` the TypeScript,
Vitest, and production-build checks, and `wasm` the `wasm32-unknown-unknown` target check. See
[Development](development.md#tests-and-feature-matrix) for runner selection options.
