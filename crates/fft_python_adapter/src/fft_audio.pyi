"""Typed interface for the embedded :mod:`fft_audio` module.

``fft_audio`` is not a pip-installable extension module. The Rust host creates
the native module and registers it in ``sys.modules`` immediately before it
runs a script, so ``import fft_audio`` works only inside an fft_audio scripting
run. This stub exists for editors and type checkers; add
``crates/fft_python_adapter/src`` to the editor/type-checker search path.

Quick start
-----------
::

    import numpy as np
    import fft_audio

    app = fft_audio.App()
    samples = np.sin(2.0 * np.pi * 10.0 * np.arange(8000) / 8000.0).astype(np.float32)
    signal = app.load_signal(samples, sample_rate=8000.0, name="10 Hz")
    spec = signal.spectrogram(show=True)
    spec.wait()
    order = spec.order_cut(fundamental_hz=10.0, orders=(1.0, 2.0, 3.0))
    order.wait()

Jobs are lazy: ``spectrogram``, ``order_cut`` and ``line_cut`` enqueue work and
return handles immediately. ``wait`` blocks until the result is ready;
``info``, ``as_numpy``, ``as_arrays``, ``to_numpy``, ``to_arrays``,
``export_csv`` and ``save_image`` wait implicitly. ``show=True`` and
``show_graph=True`` register a display immediately, before the data is ready.

Read views (``as_numpy``/``as_arrays``) are zero-copy and read-only.
``to_numpy``/``to_arrays`` return independent copies. ``as_numpy_mut`` takes an
exclusive mutable lease for result data, and ``into_numpy`` transfers ownership
and detaches the handle. Canonical displayed spectrogram data is always
read-only.

Cancellation is cooperative: the engine checks the run token whenever a script
calls back into ``fft_audio`` (including status, errors and waits). A pure
Python loop that never calls the bindings cannot be interrupted.
"""

from __future__ import annotations

from pathlib import Path
from typing import ClassVar, NoReturn, Sequence

import numpy as np
from numpy.typing import NDArray

__version__: str

class DataDetachedError(RuntimeError):
    """The handle's data was transferred to NumPy by ``into_numpy()``."""

class DataBorrowedError(RuntimeError):
    """The data is under an exclusive mutable lease or a read view is still live."""

class CancelledError(RuntimeError):
    """The script run was cancelled."""

class JobFailedError(RuntimeError):
    """An analysis or cut job failed; inspect the handle's ``error`` property."""

class Window:
    """FFT window function.

    Values are enum-like objects; compare with ``Window.HANN`` or with the
    matching string in ``.value``. ``Window("hann")`` also works.
    """

    HANN: ClassVar[Window]
    HAMMING: ClassVar[Window]
    BLACKMAN: ClassVar[Window]
    NUTTALL: ClassVar[Window]
    TRIANGULAR: ClassVar[Window]
    RECTANGULAR: ClassVar[Window]
    NONE: ClassVar[Window]
    value: str
    name: str
    def __init__(self, value: str) -> None: ...
    def __str__(self) -> str: ...

class CutAxis:
    """Axis used to lay out order cuts.

    ``TIME`` plots order cuts against time; ``RPM`` and ``FREQ`` are available
    for RPM/frequency-oriented views. The display receives the same axis value.
    """

    TIME: ClassVar[CutAxis]
    RPM: ClassVar[CutAxis]
    FREQ: ClassVar[CutAxis]
    value: str
    name: str
    def __init__(self, value: str) -> None: ...
    def __str__(self) -> str: ...

class DisplayKind:
    """Kind of display panel created by the app."""

    SPECTROGRAM: ClassVar[DisplayKind]
    ORDER_CUTS: ClassVar[DisplayKind]
    LINE_CUTS: ClassVar[DisplayKind]
    value: str
    name: str
    def __init__(self, value: str) -> None: ...
    def __str__(self) -> str: ...

class JobStatus:
    """Lifecycle state of an analysis or cut handle."""

    PENDING: ClassVar[JobStatus]
    RUNNING: ClassVar[JobStatus]
    READY: ClassVar[JobStatus]
    FAILED: ClassVar[JobStatus]
    CLOSED: ClassVar[JobStatus]
    value: str
    name: str
    def __init__(self, value: str) -> None: ...
    def __str__(self) -> str: ...

class TableInfo:
    """Metadata for a table-backed signal.

    Tables are read through the runtime; ``signal`` is the numeric column used
    for analysis, ``time`` is the optional time column, and ``notes`` records
    forward-fills, uneven time steps, or other ingestion details.
    """

    @property
    def rows(self) -> int: ...
    @property
    def columns(self) -> tuple[str, ...]: ...
    @property
    def signal(self) -> str: ...
    @property
    def time(self) -> str | None: ...
    @property
    def notes(self) -> tuple[str, ...]: ...

class AppInfo:
    """Identity of the running app.

    ``mode`` is ``"desktop"`` or ``"headless"``. It does not indicate whether a
    scripting UI is available; that is a frontend capability.
    """

    @property
    def name(self) -> str: ...
    @property
    def version(self) -> str: ...
    @property
    def mode(self) -> str: ...

class SpectrogramInfo:
    """Resolved metadata for a spectrogram analysis.

    ``frames`` and ``bins`` are the matrix dimensions; ``frame_s`` is the true
    FFT frame duration and ``hop_s`` the frame spacing. ``bin_hz`` is the
    frequency spacing and ``nyquist`` the highest displayed frequency.
    ``max_db``/``min_db`` describe the data range. ``table`` is present only
    when the signal came from a table file.
    """

    @property
    def name(self) -> str: ...
    @property
    def path(self) -> Path | None: ...
    @property
    def sample_rate(self) -> float: ...
    @property
    def frame_s(self) -> float: ...
    @property
    def frames(self) -> int: ...
    @property
    def bins(self) -> int: ...
    @property
    def bin_hz(self) -> float: ...
    @property
    def hop_s(self) -> float: ...
    @property
    def duration_s(self) -> float: ...
    @property
    def nyquist(self) -> float: ...
    @property
    def max_db(self) -> float: ...
    @property
    def min_db(self) -> float: ...
    @property
    def table(self) -> TableInfo | None: ...

class OrderCutInfo:
    """Resolved metadata for an order cut.

    ``times`` has one entry per frame. ``orders`` is the requested order list.
    ``frequencies`` has shape ``(frames, order_count)`` and gives the mapped
    frequency in Hz for each frame and order; for a constant RPM the columns
    are constant.
    """

    @property
    def name(self) -> str: ...
    @property
    def frames(self) -> int: ...
    @property
    def order_count(self) -> int: ...
    @property
    def times(self) -> NDArray[np.float64]: ...
    @property
    def orders(self) -> tuple[float, ...]: ...
    @property
    def frequencies(self) -> NDArray[np.float32]: ...

class LineCutInfo:
    """Resolved metadata for a line cut.

    ``times`` and ``frequencies`` are the sampled positions in seconds and Hz.
    ``fractions`` is the normalized 0..1 position along the line.
    """

    @property
    def name(self) -> str: ...
    @property
    def length(self) -> int: ...
    @property
    def times(self) -> NDArray[np.float64]: ...
    @property
    def frequencies(self) -> NDArray[np.float64]: ...
    @property
    def fractions(self) -> NDArray[np.float64]: ...

class Display:
    """A display panel registered by the app.

    Script-created displays use the reserved ``1_000_000+`` id range and are
    ephemeral, so they are not restored from the session. ``close()`` is
    idempotent; closing a parent also closes its child cut displays.
    """

    @property
    def id(self) -> int: ...
    @property
    def parent_id(self) -> int | None: ...
    @property
    def kind(self) -> DisplayKind: ...
    @property
    def name(self) -> str: ...
    @property
    def visible(self) -> bool: ...
    def show(self) -> None: ...
    def hide(self) -> None: ...
    def close(self) -> None: ...

class App:
    """Entry point for the current scripting run.

    ``App()`` binds to the app/runtime hosting this script. It is only valid
    while a script is running. Data handles own runtime data; displays are
    separate optional handles created by ``show_*`` calls. Closing a handle
    releases its runtime data, while displays stay open until explicitly
    closed or the app shuts down.
    """

    def __init__(self) -> None: ...
    def open_file(self, path: str | Path, *, name: str | None = None) -> Signal:
        """Load an audio or table file through the runtime.

        Unlike analysis and cuts, file loading blocks until the signal is
        decoded so metadata is available immediately. Table files use the
        configured signal/time columns and report ``SpectrogramInfo.table``.
        """

    def load_signal(
        self,
        samples: NDArray[np.float32] | NDArray[np.float64] | Sequence[float],
        *,
        sample_rate: float,
        name: str | None = None,
    ) -> Signal:
        """Copy samples into a runtime-owned signal.

        Accepts float32/float64 NumPy arrays or any finite float sequence. The
        input is copied once; mutating the Python array afterwards does not
        change the signal. Analysis starts only when ``spectrogram()`` is
        called.
        """

    def displays(self) -> tuple[Display, ...]:
        """Return a snapshot of every display currently registered by the app."""

    def wait_all(self, timeout_s: float | None = None) -> None:
        """Block until every runtime job is terminal (or the timeout expires).

        This also resolves pending display associations. It does not close
        displays or release data.
        """

    def close_all(self) -> None:
        """Close every display, releasing runtime data no longer referenced."""

    def info(self) -> AppInfo:
        """Return the app name, version and mode."""

    def version(self) -> str:
        """Return the app version string."""

class Signal:
    """Runtime-owned sample data for one signal.

    A signal keeps its samples alive for analyses and cuts. ``close()`` closes
    displays whose state references this signal and then releases the signal.
    """

    @property
    def name(self) -> str: ...
    @property
    def sample_rate(self) -> float: ...
    @property
    def sample_count(self) -> int: ...
    @property
    def duration_s(self) -> float: ...
    def spectrogram(
        self,
        *,
        fft_size: int = 4096,
        frame: int | None = None,
        pad: int | None = None,
        hop: int | None = None,
        window: Window = ...,
        freq_range: tuple[float, float] | None = None,
        db_range: tuple[float, float] | None = None,
        log_freq: bool | None = None,
        show: bool = True,
        name: str | None = None,
    ) -> SpectrogramHandle:
        """Submit a spectrogram analysis and return immediately.

        ``fft_size`` alone maps to ``frame=fft_size, pad=1``. If ``frame`` is
        also given, ``fft_size`` must be a multiple of it and the ratio becomes
        the zero-padding factor. ``hop`` defaults to the frame size. A larger
        ``fft_size`` with padding interpolates bins but does not add true
        resolution.

        ``freq_range`` sets the display's ``min_hz``/``max_hz``,
        ``db_range`` sets absolute display bounds, and ``log_freq=False``
        selects a linear frequency axis instead of the default logarithmic
        one. ``show=True`` registers a pending Spectrogram display;
        ``show=False`` is data-only, although a later child cut with
        ``show_graph=True`` publishes this spectrogram as its parent. Use
        ``SpectrogramHandle.order_cut`` or ``line_cut`` to create child cuts.
        """

    def close(self) -> None:
        """Close this signal and any displays referencing it. Idempotent."""

class SpectrogramHandle:
    """Lazy handle to a spectrogram analysis and its optional display.

    The handle is returned before analysis finishes. ``wait()`` blocks;
    ``info``, ``as_numpy`` and ``save_image`` wait implicitly. ``show=True``
    registers a display immediately in the loading state, and that display
    updates when the analysis completes.
    """

    @property
    def info(self) -> SpectrogramInfo:
        """Resolved analysis metadata; waits for the analysis."""

    @property
    def display(self) -> Display | None:
        """The display created by ``show=True``, or ``None`` for data-only."""

    @property
    def status(self) -> JobStatus:
        """Current analysis lifecycle state."""

    @property
    def error(self) -> Exception | None:
        """The analysis failure, if any."""

    def order_cut(
        self,
        *,
        fundamental_hz: float | Sequence[float] | NDArray[np.float64] | None = None,
        rpm: float | Sequence[float] | NDArray[np.float64] | None = None,
        orders: Sequence[float] = (1.0,),
        bandwidth_hz: float | None = None,
        bandwidth_orders: float | None = None,
        axis: CutAxis = CutAxis.TIME,
        show_graph: bool = True,
        name: str | None = None,
    ) -> OrderCutHandle:
        """Submit an order cut and return immediately.

        Pass either ``fundamental_hz`` or ``rpm``, not both. A scalar
        ``fundamental_hz`` is converted to a constant RPM
        (``fundamental_hz * 60``); a sequence supplies a per-frame RPM profile.
        ``orders`` are multiples of the fundamental and must be finite and
        non-zero. Use either ``bandwidth_hz`` (fixed Hz width, uniform across
        orders) or ``bandwidth_orders`` (order-relative width), not both.

        ``show_graph=True`` registers a pending Order Cuts display as a child
        of this Spectrogram; if the spectrogram was created with
        ``show=False``, the parent display is published lazily so the child has
        a parent. ``show_graph=False`` is data-only.
        """

    def line_cut(
        self,
        *,
        start: tuple[float, float],
        end: tuple[float, float],
        num_points: int = 256,
        log: bool = False,
        show_graph: bool = True,
        name: str | None = None,
    ) -> LineCutHandle:
        """Submit a line cut and return immediately.

        ``start``/``end`` are ``(time_s, frequency_hz)`` pairs. ``num_points``
        is the exact total sample count. ``log=True`` samples the frequency
        axis logarithmically. ``show_graph=True`` creates a child Line Cuts
        display.
        """

    def as_numpy(self) -> NDArray[np.float32]:
        """Return a read-only ``(frames, bins)`` zero-copy view; waits first."""

    def to_numpy(self) -> NDArray[np.float32]:
        """Return an independent writable ``(frames, bins)`` copy."""

    def as_numpy_mut(self) -> NoReturn:
        """Not supported for canonical spectrogram data; always raises."""

    def into_numpy(self) -> NoReturn:
        """Not supported for canonical spectrogram data; always raises."""

    def save_image(
        self, path: str | Path, *, width: int = 1600, height: int = 900
    ) -> Path:
        """Render the spectrogram to a PNG and return its path; waits first."""

    def wait(self, timeout_s: float | None = None) -> None:
        """Block until the analysis is ready, failed or cancelled."""

    def close(self) -> None:
        """Close the display and release the analysis. Idempotent."""

class OrderCutHandle:
    """Lazy handle to an order-cut result and its optional display.

    ``as_numpy`` returns a read-only ``(frames, orders)`` dB matrix. The
    canonical result is owned by the runtime; use ``as_numpy_mut`` for an
    exclusive mutable lease or ``into_numpy`` to transfer ownership when the
    display no longer needs it.
    """

    @property
    def info(self) -> OrderCutInfo:
        """Resolved cut metadata; waits for the cut."""

    @property
    def display(self) -> Display | None:
        """The child Order Cuts display, or ``None`` for data-only."""

    @property
    def status(self) -> JobStatus:
        """Current cut lifecycle state."""

    @property
    def error(self) -> Exception | None:
        """The cut failure, if any."""

    def as_numpy(self) -> NDArray[np.float32]:
        """Return a read-only ``(frames, orders)`` zero-copy view; waits first."""

    def to_numpy(self) -> NDArray[np.float32]:
        """Return an independent writable ``(frames, orders)`` copy."""

    def as_numpy_mut(self) -> NDArray[np.float32]:
        """Exclusive zero-copy lease; read views/info/displays are rejected.

        Other data methods raise DataBorrowedError until close(), which clears
        the lease and makes the returned array read-only.
        """

    def into_numpy(self) -> NDArray[np.float32]:
        """Unique-ownership transfer; read views/info must be closed first."""

    def export_csv(self, path: str | Path) -> Path:
        """Write the cut matrix to CSV and return its path; waits first."""

    def wait(self, timeout_s: float | None = None) -> None:
        """Block until the cut is ready, failed or cancelled."""

    def close(self) -> None:
        """Close the display and release the cut. Idempotent."""

class LineCutHandle:
    """Lazy handle to a line-cut result and its optional display.

    ``as_arrays`` returns ``(times_s, db)`` read-only views;
    ``to_arrays`` returns copies. ``to_numpy`` returns the interleaved
    ``(N, 2)`` copy. ``set_line`` replaces the line and recomputes the cut
    asynchronously while keeping the display updated.
    """

    @property
    def info(self) -> LineCutInfo:
        """Resolved cut metadata; waits for the cut."""

    @property
    def display(self) -> Display | None:
        """The child Line Cuts display, or ``None`` for data-only."""

    @property
    def status(self) -> JobStatus:
        """Current cut lifecycle state."""

    @property
    def error(self) -> Exception | None:
        """The cut failure, if any."""

    def as_arrays(self) -> tuple[NDArray[np.float64], NDArray[np.float32]]:
        """Return read-only ``(times_s, db)`` zero-copy views; waits first."""

    def to_arrays(self) -> tuple[NDArray[np.float64], NDArray[np.float32]]:
        """Return independent ``(times_s, db)`` copies."""

    def to_numpy(self) -> NDArray[np.float64]:
        """Return an independent ``(N, 2)`` ``(time_s, db)`` copy."""

    def into_numpy(self) -> NDArray[np.float64]:
        """Transfer ownership to a ``(N, 2)`` NumPy array and detach the handle."""

    def set_line(
        self,
        *,
        start: tuple[float, float],
        end: tuple[float, float],
        num_points: int | None = None,
    ) -> None:
        """Replace the sampled line and recompute asynchronously.

        ``start``/``end`` are ``(time_s, frequency_hz)``. ``num_points`` keeps
        the previous count when omitted. Call ``wait()`` before reading the new
        result.
        """

    def export_csv(self, path: str | Path) -> Path:
        """Write the line samples to CSV and return its path; waits first."""

    def wait(self, timeout_s: float | None = None) -> None:
        """Block until the cut is ready, failed or cancelled."""

    def close(self) -> None:
        """Close the display and release the cut. Idempotent."""

def app() -> App:
    """Return an :class:`App` for the current scripting run.

    Equivalent to ``App()`` and useful as a short module-level helper.
    """
