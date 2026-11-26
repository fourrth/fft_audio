#!/usr/bin/env python3
"""Open a three-tone spectrogram and order-cut display with ``fft_audio``.

The generated signal is three clean, equal-amplitude sinusoids at 10, 20 and
30 Hz. Treating 10 Hz as the fundamental (600 rpm) means orders 1, 2 and 3 land
exactly on the three tones.

The analysis uses 8192 Hz sampling with an 8192-sample frame, so every FFT bin
is exactly 1 Hz wide. With a rectangular window and whole-cycle tones, 10/20/30
Hz land on exact bins: the spectrogram shows single-bin lines and the order-cut
levels stay flat instead of varying between frames. The example also asks for a
linear 0-60 Hz display axis so the low-frequency lines are easy to see.

Desktop / interactive use
-------------------------
Open this file from the desktop Scripting panel (Load file...) and press Run.
The script registers a Spectrogram display and an Order Cuts child display,
waits for the analysis, and leaves both panels open.

Headless WebUI
--------------
::

    cargo run -p fft_desktop --no-default-features --features http,python -- \\
        --serve --port 8765 --bind 127.0.0.1 --dist web/dist \\
        --python-script examples/three_tones_example.py --workers 1

The browser shell reconciles backend displays on startup, so opening the WebUI
after the script has run still shows both panels. The desktop Scripting panel
is the interactive alternative.

Data check without a listener
-----------------------------
::

    cargo run -p fft_desktop --no-default-features --features python -- \\
        --python-script examples/three_tones_example.py --workers 1

Set ``FFT_AUDIO_CHECK_LEVELS=1`` to assert that the three order-cut levels are
within 0.5 dB of one another and that the frequencies map correctly. That check
doubles as an amplitude regression test; without the environment variable the
script only displays and prints the levels.

Set ``FFT_AUDIO_IMAGE_PATH=/tmp/three_tones.png`` to render the same display
settings to a PNG for inspection.
"""

from __future__ import annotations

import os
from pathlib import Path

import numpy as np

import fft_audio

SAMPLE_RATE = 8192.0
DURATION_S = 8.0
FFT_SIZE = 8192
HOP = 2048
FREQUENCIES = (10.0, 20.0, 30.0)
AMPLITUDE = 0.2
ORDER_BANDWIDTH_HZ = 1.0
MAX_LEVEL_SPREAD_DB = 0.5
MAX_FRAME_VARIATION_DB = 0.05


def build_signal() -> np.ndarray:
    """Return three clean, equal-amplitude sinusoids at 10/20/30 Hz."""
    sample_count = int(round(SAMPLE_RATE * DURATION_S))
    t = np.arange(sample_count, dtype=np.float64) / SAMPLE_RATE
    samples = np.zeros_like(t)
    for frequency in FREQUENCIES:
        samples += AMPLITUDE * np.sin(2.0 * np.pi * frequency * t)
    return samples.astype(np.float32)


def check_levels(order: fft_audio.OrderCutHandle) -> None:
    """Check order frequencies and amplitudes (future regression-test hook)."""
    info = order.info
    matrix = order.as_numpy()
    expected_shape = (info.frames, info.order_count)
    if matrix.shape != expected_shape:
        raise AssertionError(f"order matrix shape {matrix.shape} != {expected_shape}")

    if tuple(info.orders) != (1.0, 2.0, 3.0):
        raise AssertionError(f"unexpected order columns: {info.orders}")

    frequencies = info.frequencies[0]
    expected_frequencies = np.asarray(FREQUENCIES, dtype=np.float32)
    if not np.allclose(frequencies, expected_frequencies, rtol=0.0, atol=1e-3):
        raise AssertionError(
            f"order frequencies {frequencies.tolist()} != {expected_frequencies.tolist()}"
        )

    levels = matrix.max(axis=0)
    spread = float(levels.max() - levels.min())
    if spread > MAX_LEVEL_SPREAD_DB:
        raise AssertionError(
            "equal-amplitude orders differ by "
            f"{spread:.2f} dB (limit {MAX_LEVEL_SPREAD_DB:.2f} dB): "
            f"{levels.tolist()}"
        )

    frame_variation = matrix.max(axis=0) - matrix.min(axis=0)
    worst_variation = float(frame_variation.max())
    if worst_variation > MAX_FRAME_VARIATION_DB:
        raise AssertionError(
            "an order is not flat across frames: "
            f"{worst_variation:.3f} dB variation "
            f"(limit {MAX_FRAME_VARIATION_DB:.2f} dB)"
        )


def run() -> None:
    app = fft_audio.App()
    app_info = app.info()
    print(f"app: {app_info.name} {app_info.version} mode={app_info.mode}")

    samples = build_signal()
    signal = app.load_signal(samples, sample_rate=SAMPLE_RATE, name="three tones")
    print(
        f"signal: {signal.name} {signal.sample_rate:g} Hz "
        f"{signal.sample_count} samples ({signal.duration_s:g}s)"
    )

    spec = signal.spectrogram(
        fft_size=FFT_SIZE,
        hop=HOP,
        window=fft_audio.Window.RECTANGULAR,
        freq_range=(0.0, 60.0),
        log_freq=False,
        show=True,
        name="10/20/30 Hz tones",
    )
    spec.wait()
    spec_info = spec.info
    print(
        f"spectrogram: frames={spec_info.frames} bins={spec_info.bins} "
        f"bin_hz={spec_info.bin_hz:.4f} display={spec.display}"
    )

    image_path = os.environ.get("FFT_AUDIO_IMAGE_PATH")
    if image_path:
        output = Path(image_path).expanduser()
        output.parent.mkdir(parents=True, exist_ok=True)
        print(f"image: {spec.save_image(output, width=1600, height=900)}")

    order = spec.order_cut(
        fundamental_hz=FREQUENCIES[0],
        orders=[1.0, 2.0, 3.0],
        bandwidth_hz=ORDER_BANDWIDTH_HZ,
        show_graph=True,
        name="orders 1-3 (10/20/30 Hz)",
    )
    order.wait()
    app.wait_all()
    order_info = order.info
    levels = order.as_numpy().max(axis=0)
    print(
        f"order cut: frames={order_info.frames} orders={tuple(order_info.orders)} "
        f"display={order.display}"
    )
    print(
        "levels: "
        + ", ".join(
            f"order {index:g} ({frequency:g} Hz)={level:.2f} dB"
            for index, frequency, level in zip(
                order_info.orders, FREQUENCIES, levels.tolist(), strict=True
            )
        )
    )

    if os.environ.get("FFT_AUDIO_CHECK_LEVELS") == "1":
        check_levels(order)
        print("PASS: three_tones_example (levels checked)")
    else:
        print(
            "PASS: three_tones_example (display only; "
            "set FFT_AUDIO_CHECK_LEVELS=1 to assert levels)"
        )


if __name__ == "__main__":
    run()
