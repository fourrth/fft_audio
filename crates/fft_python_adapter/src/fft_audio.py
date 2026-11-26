"""Embedded ``fft_audio`` module shim.

In an embedded run the Rust engine creates the native module object and
registers it in ``sys.modules`` before executing the script, so ``import
fft_audio`` resolves to that native module and this source file is not
imported. It is kept next to ``fft_audio.pyi`` and ``py.typed`` so tooling and
packaging can locate the typed public surface. Add this directory to an
editor/type-checker search path so completions and docstrings come from
``fft_audio.pyi``. See that file for the full API.
"""

__all__: list[str] = []
