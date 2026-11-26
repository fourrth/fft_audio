#!/usr/bin/env python3
"""Build the optional static browser bundle.

Output: web/dist (HTML/JS/CSS plus wasm/fft_wasm*). Serve that directory from
any static host; no application server or upload endpoint is needed.
"""

from pathlib import Path
import shutil
import subprocess
import sys


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    sys.exit(1)


def run(cmd: list[str]) -> None:
    subprocess.run(cmd, check=True)


def main() -> None:
    root = Path(__file__).resolve().parent

    if shutil.which("cargo") is None:
        fail("cargo is required")

    if shutil.which("wasm-bindgen") is None:
        fail("wasm-bindgen is required (cargo install wasm-bindgen-cli)")

    if shutil.which("rustup") is None:
        fail("rustup is required")

    installed_targets = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()

    if "wasm32-unknown-unknown" not in installed_targets:
        fail(
            "the wasm32-unknown-unknown target is required "
            "(rustup target add wasm32-unknown-unknown)"
        )

    if not (root / "web" / "node_modules").is_dir():
        fail("web/node_modules is missing; run: npm --prefix web install")

    run([
        "cargo",
        "build",
        "-p",
        "fft_wasm",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ])

    out_dir = root / "web" / "public" / "wasm"
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    run([
        "wasm-bindgen",
        "--target",
        "web",
        "--no-typescript",
        "--out-dir",
        str(out_dir),
        "--out-name",
        "fft_wasm",
        str(root / "target" / "wasm32-unknown-unknown" / "release" / "fft_wasm.wasm"),
    ])

    run(["npm", "--prefix", "web", "run", "build:browser"])

    print()
    print(f"Static browser build ready: {root / 'web' / 'dist'}")
    print("Serve that directory with any static web server.")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as exc:
        sys.exit(exc.returncode)