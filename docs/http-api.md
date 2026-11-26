# Headless HTTP/WebSocket API

Start the server with an `http`-feature build:

```sh
cargo run -p fft_desktop --no-default-features --features http -- \
  --serve --port 8765 --bind 127.0.0.1 --dist web/dist
```

The listener binds `127.0.0.1:8765` by default; `--bind` may select any local IP or `IP:PORT`. JSON uses camelCase;
errors are `{"error": "..."}` with a non-2xx status.

## Routes

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/app/info` | `{name, version, mode, capabilities}`; capabilities include displays, scripting (`null` over HTTP), and compiled table formats |
| POST | `/api/app/ping` | `{nonce?, at?}` round-trip |
| GET/PUT | `/api/settings` | Read/replace `Settings` |
| POST | `/api/settings/defaults` | Save new-Spectrogram analysis/display defaults |
| GET | `/api/config/paths` | Config/settings/session/themes paths |
| POST | `/api/open-path` | Desktop-only; HTTP returns an error |
| GET | `/api/theme?name=dark` | Resolved theme |
| GET | `/api/themes` | Theme entries, preview swatches, parse errors |
| POST | `/api/themes/duplicate` | `{source,name}` |
| GET/PUT | `/api/session` | `SessionDocument` |
| POST | `/api/upload` | Multipart field `file`; returns `{path,name}` |
| POST | `/api/spectrogram/open` | `{displayId,path,settings?}` starts analysis; returns `{displayId,analysisId}` with the caller-owned display id |
| GET | `/api/spectrogram/{id}` | base64 i16 grid payload + metadata |
| GET | `/api/spectrogram/{id}/image` | 1600×900 PNG download |
| POST | `/api/analysis/cancel` | `{displayId}` |
| GET | `/api/displays` | Live registry metadata, sorted by id |
| GET | `/api/displays/{id}` | One `DisplayInfo` (`id,parent,kind,name,state,visible,ephemeral`) |
| POST | `/api/displays/close` | `{id}` closes the display subtree; idempotent when absent |
| POST | `/api/displays/spectrogram` | `{analysisId,ephemeral?}` creates a backend-owned `1_000_000+` display; ready analysis required |
| POST | `/api/cuts/order` | `{parentId,settings}` |
| POST | `/api/cuts/order/image` | `{parentId,settings,rpm?}` → order-cut PNG (an empty plot when the parent analysis is not ready) |
| POST | `/api/cuts/line` | `{parentId,settings,lines}` |
| POST | `/api/cuts/csv` | `{cuts}` → CSV download |
| POST | `/api/plot` | `{path,layout}` maps a table onto plot series: `layout` is `columns` (first column = shared x, other numeric columns = lines) or `pairs` (`x1,y1,x2,y2,...`); returns `{xLabel,series,rows,notes}` |
| GET | `/ws` | WebSocket events |

The HTTP surface executes no code and reads/writes no script files. Scripting is available only through the local
`--python-script` CLI path or the desktop Scripting panel; both run in-process and are never exposed to browser
clients.

## WebSocket events

Each message is a JSON object with a `type` field:

* `analysis.progress`
* `analysis.done`
* `analysis.error`
* `display.open` `{display}`
* `display.updated` `{display}`
* `display.closed` `{id,children}`

CLI script stdout/stderr does not go over the WebSocket ring. Display adoption is reconciled on browser startup with
`GET /api/displays`, so displays created before the UI connects still appear.

Tauri uses the same display contract through `display_list`, `display_get`, `display_close`,
`display_show_spectrogram`, and `display-open` / `display-updated` / `display-closed` events.

## Limits and security

* There is no authentication or TLS. Binding beyond loopback (for example `--bind 0.0.0.0`) exposes uploads,
  settings, and analysis data to anyone who can reach the port; do so only on a trusted network.
* Browser uploads are limited to 512 MiB.
* CORS accepts loopback origins only. Same-origin requests are unaffected, including a page the server itself
  serves over a non-loopback address; a remote web page cannot call the API cross-origin.
* Uploads and spectrogram opens read/write their local analysis files by design.
* No editor, script file access, or HTTP execution path exists in the WebUI.
