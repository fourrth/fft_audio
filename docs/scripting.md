# Scripting

Python is the first scripting engine. The scripting contract (`fft_script`) and the CLI composition are
language-neutral; a future language would add another engine/adapter and feature rather than changing the app contract.

## Desktop Scripting panel

The Launcher's **Scripting** tile is present when the desktop app is built with `tauri_python` (the default). A
`tauri`-only build hides the tile and reports `capabilities.scripting = null`.

The panel runs Python **in-process** through `fft_python_adapter`:

* choose a local `.py` file; the editor is a plain text editor, not an IDE;
* **Save** writes the file; `Ctrl/Cmd+S` saves; **Reload** re-reads it; a dirty buffer prompts before discarding;
* **Run** executes the saved file; **Stop** requests cooperative cancellation;
* stdout, stderr, and engine logs stream into a **Console** child display;
* **Auto-run on save** watches the file with `tauri-plugin-fs` and runs the on-disk file;
* **Load example…** lists every `.py` file under the repository's `examples/` directory. The desktop adapter embeds
  them at compile time, copies the selected file into the config directory's `scripting-examples/`, and flattens
  nested example directories to a single filename.

Scripts are trusted local code. There is no sandbox. Python 3.10+, NumPy, and development headers are required for
`tauri_python`.

## CLI scripting

Build with the `python` feature. This is independent of the GUI features. The CLI path uses the same
`fft_python_adapter` engine as the desktop panel:

```sh
cargo run -p fft_desktop --no-default-features --features python -- \
  --python-script examples/harmonics_chirp_example.py --workers 1
```

Without `--serve`, the process opens no listener and exits after the scripts. With `http` enabled, the server starts
first and keeps serving after the scripts finish:

```sh
cargo run -p fft_desktop --no-default-features --features http,python -- \
  --serve --port 8765 --dist web/dist \
  --python-script examples/harmonics_chirp_example.py
```

`--python-script` is repeatable and runs sequentially in one interpreter/app. The first failure stops later scripts
and exits non-zero. `--exit-after-script` stops the server after the last script. The WebUI never submits,
reads, or executes scripts; browser clients only see displays and analysis events.

`--python-wait` waits for Enter; `--python-delay <SECONDS>` waits a fixed time. Both happen after the WebUI prints
its URL and before the first script, so a browser can connect before scripted displays are created. Backend displays
are also reconciled by the web shell on startup.

## Lease behavior

The root `fft_desktop` owns the app host and mints the Python lease. The Python adapter receives one
adapter-lifetime lease; sequential script runs receive clones of that lease. The engine drops its run handle when the
script exits. The Tauri adapter's `tauri_python` path receives a separate scripting lease from the root; it never
mints one itself.

## Python API

The typed module is created in-process at runtime. Type stubs and docstrings live at
`crates/fft_python_adapter/src/fft_audio.pyi`; `pyrightconfig.json` adds that directory to Pylance's search path.

The API exposes typed handles for the app, signals, spectrograms, order cuts, line cuts, and displays. NumPy data
views are read-only by default; explicit copies are available, and mutable views require exclusive ownership. Waits
release the GIL and check cooperative cancellation.

## Examples

* `examples/harmonics_chirp_example.py` — in-memory chirp plus 50 Hz harmonics, order/line cuts, CSV/PNG exports.
* `examples/three_tones_example.py` — 10/20/30 Hz tones with 1 Hz FFT bins; opens a spectrogram and order cuts with
  crisp lines and flat orders. Set `FFT_AUDIO_CHECK_LEVELS=1` to assert the three levels are within 0.5 dB, and
  `FFT_AUDIO_IMAGE_PATH=/tmp/tones.png` to render the same display settings to a PNG.

Output from an example can be directed with `FFT_AUDIO_SCRIPT_WORKDIR`.
