#!/usr/bin/env python3
"""In-process CLI Python example for the ``python`` Cargo feature.

Run a local script without any HTTP listener::

    cargo run -p fft_desktop --no-default-features --features python -- \\
        --python-script examples/harmonics_chirp_example.py --workers 1

The script builds a chirp plus a 50 Hz harmonic stack in memory, analyses it
through the shared runtime, reads an order cut at orders 1/2/3 (fixed 8 Hz
bandwidth), and samples a line cut with an exact ``num_points``. It exports
two CSVs and a PNG, prints a compact summary, and returns non-zero on failure.

NumPy is required by the embedded ``fft_audio`` array API. The same script also
runs from the desktop Scripting panel, which uses the same in-process engine.
"""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
import traceback
from pathlib import Path

import numpy as np

import fft_audio

SAMPLE_RATE = 8000.0
DURATION_S = 6.0
FUNDAMENTAL_HZ = 50.0
ORDERS = (1.0, 2.0, 3.0)
BANDWIDTH_HZ = 8.0
LINE_POINTS = 37


def build_signal() -> np.ndarray:
    """Return a float32 harmonic stack plus a low 10 -> 20 Hz chirp."""
    sample_count = int(round(SAMPLE_RATE * DURATION_S))
    t = np.arange(sample_count, dtype=np.float64) / SAMPLE_RATE
    samples = (
        np.sin(2.0 * np.pi * FUNDAMENTAL_HZ * t)
        + 0.5 * np.sin(2.0 * np.pi * 2.0 * FUNDAMENTAL_HZ * t)
        + 0.25 * np.sin(2.0 * np.pi * 3.0 * FUNDAMENTAL_HZ * t)
    )
    # Keep the chirp below the 50 Hz order band so it only adds broadband
    # content; the equal-amplitude relationship of the harmonics is exact.
    chirp_hz = 10.0 + 10.0 * (t / DURATION_S)
    samples += 0.15 * np.sin(2.0 * np.pi * np.cumsum(chirp_hz) / SAMPLE_RATE)
    return samples.astype(np.float32)


def require(condition: bool | np.bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def verify(workdir: Path) -> None:
    samples = build_signal()
    app = fft_audio.App()
    app_info = app.info()
    print(f"app: {app_info.name} {app_info.version} mode={app_info.mode}")

    signal = app.load_signal(samples, sample_rate=SAMPLE_RATE, name="harmonics-chirp")
    require(signal.sample_count == samples.size, "signal sample count changed")
    require(abs(signal.duration_s - DURATION_S) < 1e-9, "signal duration changed")

    spec = signal.spectrogram(
        fft_size=2048,
        hop=512,
        window=fft_audio.Window.HANN,
        show=False,
        name="harmonics-chirp",
    )
    spec.wait()
    info = spec.info
    require(info.frames > 0 and info.bins > 0, "spectrogram is empty")

    grid = spec.as_numpy()
    require(grid.dtype == np.float32, f"spectrogram dtype is {grid.dtype}")
    require(grid.shape == (info.frames, info.bins), f"grid shape {grid.shape} != info shape")
    require(not grid.flags.writeable, "canonical spectrogram view must be read-only")
    require(bool(np.isfinite(grid).all()), "spectrogram contains non-finite levels")

    order = spec.order_cut(
        fundamental_hz=FUNDAMENTAL_HZ,
        orders=list(ORDERS),
        bandwidth_hz=BANDWIDTH_HZ,
        show_graph=False,
        name="harmonics",
    )
    order.wait()
    order_info = order.info
    matrix = order.as_numpy()
    require(matrix.dtype == np.float32, f"order matrix dtype is {matrix.dtype}")
    require(
        matrix.shape == (order_info.frames, len(ORDERS)),
        f"order matrix shape {matrix.shape} != ({order_info.frames}, {len(ORDERS)})",
    )
    require(not matrix.flags.writeable, "order matrix view must be read-only")
    require(bool(np.isfinite(matrix).all()), "order matrix contains non-finite levels")
    require(tuple(order_info.orders) == ORDERS, f"order columns are {order_info.orders}")

    levels = matrix.max(axis=0)
    # Amplitudes 1, 1/2, 1/4 imply roughly 0, -6 and -12 dB at 50/100/150 Hz.
    require(
        levels[0] > levels[1] + 4.0,
        f"order 1 is not clearly above order 2: {levels.tolist()}",
    )
    require(
        levels[1] > levels[2] + 4.0,
        f"order 2 is not clearly above order 3: {levels.tolist()}",
    )

    order_csv = order.export_csv(workdir / "harmonics_order_cut.csv")
    require(order_csv.exists() and order_csv.stat().st_size > 0, "order CSV was not written")

    line = spec.line_cut(
        start=(DURATION_S * 0.1, FUNDAMENTAL_HZ),
        end=(DURATION_S * 0.9, FUNDAMENTAL_HZ),
        num_points=LINE_POINTS,
        show_graph=False,
        name="fundamental line",
    )
    line.wait()
    line_times, line_db = line.as_arrays()
    require(
        line_times.shape == (LINE_POINTS,),
        f"line cut returned {line_times.shape[0]} points, expected {LINE_POINTS}",
    )
    require(line_db.shape == (LINE_POINTS,), f"line dB shape is {line_db.shape}")
    require(
        line_times.dtype == np.float64 and line_db.dtype == np.float32,
        f"line dtypes are {line_times.dtype}/{line_db.dtype}",
    )
    require(
        not line_times.flags.writeable and not line_db.flags.writeable,
        "line views must be read-only",
    )
    require(np.isfinite(line_db).all(), "line cut contains non-finite levels")
    require(
        np.allclose(line.info.frequencies, FUNDAMENTAL_HZ, atol=1e-9),
        "line cut frequencies do not match the requested 50 Hz line",
    )
    require(
        line_times[0] < line_times[-1],
        "line cut times are not increasing along the requested line",
    )

    line_csv = line.export_csv(workdir / "harmonics_line_cut.csv")
    require(line_csv.exists() and line_csv.stat().st_size > 0, "line CSV was not written")

    png = spec.save_image(workdir / "harmonics_spectrogram.png", width=640, height=360)
    require(png.exists(), "spectrogram PNG was not written")
    data = png.read_bytes()
    require(
        len(data) > 8 and data[:8] == b"\x89PNG\r\n\x1a\n",
        "spectrogram export is not a PNG",
    )

    print(
        "summary: "
        f"frames={info.frames} bins={info.bins} "
        f"orders={'/'.join(f'{order:g}' for order in ORDERS)} "
        f"levels={'/'.join(f'{level:.1f}' for level in levels)} dB "
        f"line_points={line_times.size} "
        f"files={order_csv.name},{line_csv.name},{png.name}"
    )
    print("PASS: harmonics_chirp_example")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workdir",
        type=Path,
        default=os.environ.get("FFT_AUDIO_SCRIPT_WORKDIR"),
        help=(
            "keep exported CSV/PNG files here (default: "
            "FFT_AUDIO_SCRIPT_WORKDIR or a temporary directory)"
        ),
    )
    args = parser.parse_args()

    temporary: tempfile.TemporaryDirectory[str] | None = None
    if args.workdir is None:
        temporary = tempfile.TemporaryDirectory(prefix="fft-harmonics-chirp-")
        workdir = Path(temporary.name)
    else:
        workdir = args.workdir
        workdir.mkdir(parents=True, exist_ok=True)

    try:
        verify(workdir)
        return 0
    except Exception as error:  # noqa: BLE001 - report every failure clearly
        print(f"FAIL: {error}", file=sys.stderr)
        traceback.print_exc()
        return 1
    finally:
        if temporary is not None:
            temporary.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
