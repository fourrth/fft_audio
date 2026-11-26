# Architecture

`fft_audio` is one product binary, `fft_desktop`, with transport adapters around one shared application model.

```
fft_core        DSP, decoding, tables, cuts, settings, sessions, themes
     │
fft_runtime     runtime contracts, CoreRuntime, scheduler, jobs
     │
fft_app         FftApp, AppLease, display/event contracts
     │
fft_app_host    concrete AppHost, display registry, lease minting
     │
     ├── fft_http_adapter     HTTP/WebSocket adapter
     ├── fft_tauri_adapter    Tauri IPC adapter
     └── fft_python_adapter   embedded CPython/NumPy engine
     │
fft_desktop     root composition, GUI/CLI mode selection, process exit
```

`fft_test` provides the shared test doubles. `fft_script` is the language-neutral scripting contract.
`fft_shipping` is a build/shipping tool, not part of the running app.

## Crates

| Crate | Responsibility |
|---|---|
| `fft_core` | UI-agnostic DSP, audio/table decoding, spectrogram computation and rendering, cut maths, settings, sessions, themes, example-script helpers. No Tauri/HTTP dependencies. |
| `fft_runtime` | `FftRuntime` data/compute/job contracts, `CoreRuntime`, owned signals/analyses/cuts, bounded worker scheduler, typed IDs/errors, progress/cancel events, `MockRuntime`. |
| `fft_app` | App contract: `FftApp`, scoped `AppLease`, display records/types/registry, app events, app info/capabilities. No concrete host and no transport types. |
| `fft_app_host` | The concrete `FftApp` implementation: `AppHost`. Owns the display registry, backend display-id allocation, lifecycle, persisted-session seeding, and is the only source of leases. |
| `fft_test` | `TestApp`, `TestHost`, `TestLease`, recording, failure injection, and lease tracking for adapter/product tests. |
| `fft_script` | Language-neutral `FftScript` contract, script source/events, and `MockScript`/test helpers. No language runtime. |
| `fft_wasm` | Optional `wasm-bindgen` bindings over `fft_core` for the static browser build. Lean text-table features, in-memory decode/analyse/cuts/render only; no host paths or scripting. |
| `fft_http_adapter` | Axum HTTP/WebSocket adapter over one server-lifetime lease. Routes, static frontend serving, uploads, display/analysis/cut endpoints, WS events. No script execution surface. |
| `fft_tauri_adapter` | Tauri IPC adapter over a GUI lease plus an optional separate scripting lease/engine. Commands, DTOs, event translation, file helpers, scripting bridge, and compile-time bundled Python examples. Never owns the host or mints leases. |
| `fft_python_adapter` | Optional CPython/NumPy engine implementing `FftScript`. Typed `fft_audio` Python API, NumPy views, cancellation, `SystemExit`, per-run lease lifecycle. Enabled with the `python` feature. |
| `fft_desktop` | The product binary. Owns `AppHost`, mints every lease, selects GUI vs CLI, installs Tauri/HTTP/Python adapters, owns Tauri config/assets, and shuts everything down. |
| `fft_shipping` | CLI meta-tool that runs cargo jobs from `shipping.toml` or ad-hoc specs and copies artifacts into output folders. |
| `fft_benchmarks` | Criterion benchmarks for FFT, spectrogram analysis, rendering, and matched NumPy comparisons. Not part of the running app or shipping binary. |

## Leases

Only `fft_desktop` constructs `AppHost` and calls `lease()`. Each adapter receives its own `Arc<dyn AppLease>` for its
lifetime:

* HTTP: one lease for the server lifetime;
* Tauri: one GUI lease, plus a separate Python lease when `tauri_python` is compiled;
* CLI Python: one lease for the Python adapter, shared by sequential script runs;
* nested adapters receive or forward a root-granted lease; they never mint one.

Adapters borrow `AppLease` and must not hold `Arc<AppHost>`. Shutdown order is scripts/server → adapter leases →
`AppHost`/runtime.

## Scripting

`fft_script` is the language-neutral contract (`FftScript`, script source/events). The optional engine lives in
`fft_python_adapter`: it owns CPython/NumPy, implements `FftScript`, and drives the app through a scoped lease.
The desktop (`tauri_python`) and the CLI (`python` + `--python-script`) share that engine. The HTTP adapter exposes
no scripting surface, so a WebUI client cannot execute scripts or read script files.

## Product modes

`fft_desktop` features:

| Feature | Build |
|---|---|
| `tauri_python` (default) | Tauri GUI with the embedded Python scripting UI |
| `tauri` | Tauri GUI without scripting |
| `http` | `--serve` headless WebUI (binds `127.0.0.1` by default) |
| `python` | CLI `--python-script` adapter |

`tauri` and `tauri_python` are mutually exclusive. `http` and `python` can be combined with each other and with GUI
features. Without any GUI feature or CLI adapter action the binary is inert.

Mode selection:

* GUI feature + no CLI action → GUI;
* `--headless` → no GUI; CLI composition;
* `--serve` → HTTP composition; no GUI;
* `--python-script` → CLI/scripting composition; no GUI.

`--headless` with no adapter flags builds and shuts down the host without opening a listener.

`--bind` chooses where the WebUI listens: `127.0.0.1:8765` by default, any local IP or `IP:PORT` otherwise. Binding a
non-loopback address is the operator's choice; there is no authentication, so the server logs a warning when it is
reachable beyond loopback.

## Frontend/build boundary

`web/` is one Vite/React/TypeScript UI used by both Tauri and HTTP. It talks through a typed `Transport`, never
directly to Rust internals. Tauri maps the transport to IPC commands/events; HTTP maps it to `/api/*` and `/ws`.

The product build script is gated:

* `webui` (enabled by `http` and both Tauri features) builds stale `web/dist` when `node_modules` exists;
* GUI builds run `tauri_build` after web assets are ready;
* `FFT_SKIP_WEB_BUILD=1` skips web generation.

## Dependency rules

* `fft_core` and `fft_runtime` must not depend on app/adapters/UI/frameworks.
* `fft_app` must not depend on Tauri, HTTP, Python, CLI, or the concrete host.
* `fft_app_host` must not depend on adapters.
* Adapters do not depend on each other.
* `fft_desktop` is the only composition root and the only lease minter.
* `fft_script` stays language-neutral; engines live in engine adapters.
