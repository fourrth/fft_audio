# UI notes

The shared React UI is used by the Tauri desktop app and the headless WebUI. Desktop-only features are gated by
`capabilities.scripting` and native dialogs.

## Launcher and displays

Every new tab starts as a **Launcher**. It offers root displays only:

* **Spectrogram** — opens a Spectrogram in that tab.
* **Recorder** — records the microphone and opens the take in a Spectrogram.
* **Scripting** — only in `tauri_python` builds; opens the editor-first Python workspace.

Order Cuts and Line Cuts are child displays. Use **Cuts → Order Cuts / Line Cuts** or the plot context menu to create
them. Children of the same Spectrogram share one companion tab group instead of creating a new split each time. They
are not Launcher tiles. Generic **Plots** are also opened from the **File** menu rather than a Launcher tile.

The top bar uses a conventional **File / View / Layout / Cuts / Help** menu bar; `Ctrl+K` still opens the command
palette. Menu groups and actions follow the active display: **Cuts** is hidden unless it applies, and the Inspector
is unavailable for scripting displays. The **+** tab action always opens a new Launcher tab immediately to the right
of the active tab and appears in every display group.

Displays are Dockview panels. Tabs support drag, group, split, resize, and middle-click close. The tab context menu
offers **Inspector**, **Duplicate**, **Rename**, and **Close**. Closing a Spectrogram with child cuts asks for
confirmation; confirming shows an 8-second **Undo** toast.

Backend displays start at id `1_000_000`; UI displays use lower ids. The shell subscribes to `display.open`,
`display.updated`, and `display.closed`, fetches `display.list` after session restore, and upserts panels by
`panel-${id}`. Backend displays created before the browser connects are adopted on startup.

## Spectrogram

An empty Spectrogram loads files with a single **Browse…** action. The native picker defaults to **All Files** (every
audio and data-table extension this build accepts) and can be narrowed to Audio or Data tables. **Enter a path** is
still available for typed paths, and the last five recent files stay visible at the bottom of the empty state.

Header actions:

* **Log / Linear** frequency axis.
* **Grid lines** toggles the tick-aligned plot grid; the choice persists with the display.
* **Fit view** (`F`).
* **Export image** — quick-exports the visible heatmap viewport as PNG, using the current frequency/color
  settings. Axes, grid, overlays, and header chrome are not included.
* **Re-analyse** and **Cancel** during analysis.
* **Toggle inspector** (`P`).

**File → Export image** runs the same quick export for the active plot. Order Cuts and Line Cuts have an
**Export image** action on the plot itself; their image contains the visible plot area (axes, ticks, grid, curves)
plus the legend, without panel header/action chrome.

The **+** action lives in the tab group, not the display header, and always opens a new Launcher tab.

Plot interactions:

* wheel pans frequency; Shift+wheel pans time; Ctrl/Cmd+wheel zooms around the pointer;
* double-click the plot background to fit the view;
* left-drag selects and zooms a region;
* Shift-drag, middle-drag, and touch-drag pan;
* line-cut endpoints snap to horizontal or vertical within 2°;
* double-click a line cut to toggle it horizontal/vertical; toggling restores the previous orientation exactly;
* `Hz`/`s` axes show ticks at the live view bounds, with grid lines that the header **Grid lines** toggle hides;
* hover shows crosshair and `t` / `f` / `dB` values;
* the right color bar has draggable/keyboard-adjustable max/min dB handles and a reset control;
* **Maximum frequency** is a hard view cap clamped to Nyquist;
* the bottom mini-map shows the whole file and current viewport; click/drag to move, reset to fit.

Analysis jobs show frames done / total and ETA, and can be cancelled.

## Order Cuts

* Cuts come from `[order_cuts]` defaults, the cut editor, or the parent Spectrogram context menu.
* Header x-axis: **RPM**, **Time**, or **Order freq**.
* Header **Grid lines**, **Export image**, **Export CSV**, and **Toggle inspector** share the Spectrogram's top-right
  action placement. Right-click the plot for **Reset view**, **Export image**, and **Add order cut**.
* A floating legend lists cuts with theme `plot_palette` colors; click a row to show/hide.
* Click a curve to select it; actions are **Delete**, **Flip** (disabled for orders), and **Edit numbers…**.
* The plot fills its child window and clips curves to the axes. Left-drag the curve area to zoom to that region;
  double-click the background, or choose **Reset view**, to fit all cuts again.
* RPM tracking can be constant, ramped, or loaded from a `time_s,rpm` CSV profile.
* The parent Spectrogram shows a tracking chip and dashed order guides for constant/ramp profiles. Host-side CSV
  profiles are not rendered as guides.
* CSV export is available from the header and command palette.

## Line Cuts

* Add lines from the parent Spectrogram context menu: **Add line here**. The line cuts child reuses its existing window.
* Select a curve and use **Delete**, **Flip**, or **Edit numbers…** (`t0/f0/t1/f1`).
* Axis selector: **Auto**, **Time**, or **Freq**.
* Header **Grid lines**, **Export image**, **Export CSV**, and **Toggle inspector** share the Spectrogram's top-right
  action placement. Right-click a plot or a selected line for the common plot actions.
* Drag endpoints on the cut plot or the parent Spectrogram overlay to rotate/resize; drag the body to move.
* Line frequencies are stored in Hz: on a log frequency axis endpoint drags stop at the axis's 10 Hz floor, while a
  linear axis allows lines all the way down to 0 Hz (and **Add line here** follows the clicked frequency).
* The plot fills its child window and clips curves to the axes. Left-drag the curve area to zoom to that region;
  double-click the background, or choose **Reset view**, to fit all cuts again.
* Degenerate lines show a dismissible warning rather than a blank plot.
* CSV export and legend visibility match Order Cuts.

## Plot

* **File → New Plot…** opens a CSV/table as a generic plot with a shared x axis: the first column is x and every
  remaining numeric column is one line. **File → New Plot (x/y pairs)…** reads interleaved `x1,y1,x2,y2,...`
  columns, where each `(x, y)` pair is one line. Column headers label the axes and the legend.
* Plots are top-level displays; they are not Launcher tiles. The header **Shared x / x/y pairs** control reinterprets
  the same file without reopening it.
* Interactions match Order Cuts and Line Cuts: the plot fills its window and clips lines to the axes, the legend
  toggles line visibility, drag zooms to a region, and double-click or **Reset view** fits the data again.
* **Grid lines** and **Export image** are in the header and the plot context menu; image export saves `plot.png`.
  Skipped rows and ignored columns appear as a dismissible warning.
* Unreadable tables (for example a non-numeric x column) show the error with **Try again**.
* Plots need a readable file path; they are not offered in the static browser/WASM build.

## Recorder

* The **Recorder** Launcher tile replaces that Launcher tab with a recording display.
* **Start recording** requests microphone access; **Stop recording** saves the take and replaces the Recorder tab with a
  Spectrogram of it. The display shows the elapsed time and the active input label.
* Microphone processing (echo cancellation, noise suppression, auto gain) is disabled so the analysed spectrum is the
  unmodified input.
* Takes are saved as lossless 16-bit PCM WAV (mono at the microphone's sample rate, about 5.8 MB per minute at
  48 kHz) through the transport's `recording.save`: the headless WebUI uploads them for server-side analysis, the
  desktop app writes them to `<config>/recordings/`, and the static browser build keeps them in its worker file
  store. No audio is sent anywhere except the configured local backend.
* Denied or unavailable microphones show an inline error with the reason; closing the tab during a take discards it.
* Desktop microphone capture is enabled for the app's WebView: Linux WebKitGTK enables media streams and allows
  audio-only permission requests, macOS builds carry an `NSMicrophoneUsageDescription`, and Windows WebView2 uses its
  normal permission prompt. The OS may still ask for microphone permission the first time.

## Inspector

`P` toggles a docked Inspector for the active display. It opens to the right at 28% width with a 300 px minimum;
below 900 px it docks as a tab.

Cards:

* **Input** — file/table path and data source.
* **Analysis** — resolution presets (high time, balanced, high frequency), window, and filters.
* **Smoothing** — time smoothing.
* **Display** — log/linear, grid lines, frequency caps, dB range, color floor/ceiling.
* **Advanced** — manual frame/hop/pad. Data-table inputs also show signal column, time column, sample rate, and
  remove mean; those options are hidden for audio files.

The card set follows the display. A Launcher shows **Input** and **Browse…** only. Scripting displays show no
analysis/display properties. Order Cuts and Line Cuts open their own Inspector, keyed by display, with editable
**Cut tracking** settings (axis, tracking/source, widths, snap/peak options) and a **Display** grid toggle. The full
FFT analysis cards are Spectrogram-only.

Resolution presets derive frame, hop, and pad from the current sample rate (about 10 ms, 50 ms, and 200 ms windows).
They stay in the Analysis card; manual sample counts remain in Advanced. The advanced table options are only shown
when the analysis came from a table (or the chosen path has a table extension).

The filter box searches settings. Display-only changes apply immediately and persist. DSP changes are staged; the
header shows a warning dot and a sticky footer offers **Revert** and **Apply & re-analyse**. The star action saves the
staged analysis/display settings as defaults for new Spectrograms.

**Color range** is the span below the loudest bin. **Color floor** and **color ceiling** pin absolute dB ends and
override the relative range; use **Use range** to clear both and return to the range mode.

## Settings

`Ctrl+,` opens Settings:

* **General** — restore last session.
* **Appearance** — UI scale 75–200%, reduce motion, high contrast, theme grid.
* **Keyboard** — searchable shortcut table.
* **Files** — config/settings/session/theme paths and desktop **Open** buttons.
* **Themes** — installed/built-in themes, parse errors, reload, duplicate current theme.
* **About** — version, mode, Tauri/Dockview/React/Rust stack.

Appearance changes apply immediately. Reduced motion sets transitions to 0 ms. High contrast strengthens borders and
secondary text. Theme selection injects the TOML theme variables at runtime.

## Commands and shortcuts

`Ctrl+K` opens the command palette. `F1` opens the shortcut reference.

| Shortcut | Action |
|---|---|
| `Ctrl+T` | New Spectrogram |
| `Ctrl+W` | Close active tab (child-aware confirmation) |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next/previous tab |
| `Ctrl+O` | Open file |
| `Ctrl+K` | Command palette |
| `Ctrl+,` | Settings |
| `P` | Toggle Inspector |
| `F` | Fit active Spectrogram |
| `M` | Maximize/restore active group |
| `Esc` | Cancel/close palette, dialogs, context menus |
| `F1` | Shortcut modal |

Context menus exist on tabs, Launcher tiles, and every plot. Plot context menus share **Reset view** and **Export image**, then add display-specific actions such as **Add line here**.

## Data tables

Supported formats: `csv`, `tsv`, `tab`, `txt`, `dat`, `json` (array of row objects), `ndjson`/`jsonl`, plus
`parquet`/`pq` and `arrow`/`ipc`/`feather` when compiled.

Delimited files use a header row. `.tsv`/`.tab` use tabs; `.csv`/`.txt`/`.dat` sniff comma, tab, semicolon, and pipe,
then fall back to comma. Empty headers become `column_N`; duplicates get `_2`, `_3`, etc. Short rows are padded; rows
with too many fields are rejected. Quoted fields and CRLF are supported.

One numeric column becomes the signal; non-numeric and Arrow bool/date/decimal/string columns are text. Configure the
signal/time column, sample rate, and **Remove mean** in **Inspector → Advanced** or `[analysis.data]` in settings.

## Themes

Built-in themes live in `crates/fft_core/assets/themes/{dark,light}.toml`. User themes are TOML files in the platform
config directory's `themes/` folder or the headless config directory.

```toml
name = "My Ocean"
extends = "dark"
appearance = "dark"
plot_palette = [
  "#56b4e9", "#e69f00", "#009e73", "#cc79a7",
  "#f0e442", "#0072b2", "#d55e00", "#e0e0e0",
]

[colors]
accent-primary = "#00e5ff"
bg-canvas = "#071018"

[status.info]
bg = "rgba(0, 229, 255, 0.14)"
fg = "#8ef6ff"
```

A theme may `extends` another. The validator checks required tokens, the 8-color palette, color syntax, and WCAG
text contrast (at least 4.5:1, including translucent status backgrounds composited over the panel). Invalid user
themes are listed with parse errors and do not replace the active theme.

## Sessions and config

Desktop config: `<platform config>/com.fft-audio.desktop/`. CLI/headless: `<config>/fft_audio/` or `--config-dir`.
Both contain `settings.toml`, `session.json`, and `themes/`.

`session.json` stores Dockview layout, display metadata, recent files, and layout presets. Backend displays with
`state.ephemeral === true` are excluded from session saves. On restore, file-backed backend Spectrograms are
recreated as user displays and re-analysed by path; pathless backend/script displays are dropped.

`settings.toml` holds `[analysis]`, `[analysis.data]`, `[display]`, `[order_cuts]`, `[order_cuts.rpm]`,
`[line_cuts]`, `[session]`, and `[appearance]` defaults. Missing values use built-in defaults. Settings files from
older versions still load; unknown tables are ignored.

## Responsive behavior

* `< 900 px` hides metadata chips and collapses secondary header actions.
* `< 600 px` uses a single-column Launcher and horizontal Settings rail.
* Touch/coarse-pointer controls use at least 44×44 px hit targets.
* Plots/colorbars/cuts use `PointerEvent`, and CSS respects safe-area insets.
* The project is desktop/headless only at present; it has not been packaged or tested for Android/iOS.
