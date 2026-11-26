#!/usr/bin/env python3
"""fft_audio development test runner.

Runs the Rust feature matrix, frontend checks, and Playwright from one
command and prints a pass/fail summary grouped by test area. Python standard
library only; run from any directory.

Examples:
    python3 scripts/dev.py                    # everything
    python3 scripts/dev.py frontend           # typecheck + unit + build
    python3 scripts/dev.py rust -v            # Rust matrix, full output
    python3 scripts/dev.py frontend e2e       # UI change with browser tests
    python3 scripts/dev.py rust/core          # one suite
    python3 scripts/dev.py --skip e2e         # everything except Playwright
    python3 scripts/dev.py --list
"""

from __future__ import annotations

import argparse
import collections
import difflib
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Iterable, Sequence

ROOT = Path(__file__).resolve().parents[1]
WEB = ROOT / "web"

AREA_ORDER = ("rust", "frontend", "e2e", "wasm")
AREA_LABELS = {
    "rust": "Rust",
    "frontend": "Frontend",
    "e2e": "E2E",
    "wasm": "WASM",
}
AREA_DESCRIPTIONS = {
    "rust": "formatting, workspace check, and the full feature-matrix tests",
    "frontend": "typecheck, production build, and Vitest unit tests",
    "e2e": "Playwright browser tests (builds web/dist first)",
    "wasm": "wasm32 target check; skipped when the target is not installed",
}

EXCERPT_LINES = 20
EXCERPT_WIDTH = 240
CAPTURE_LINES = 800
SEPARATOR_WIDTH = 72


class Style:
    BOLD = "\033[1m"
    DIM = "\033[2m"
    GREEN = "\033[32m"
    RED = "\033[31m"
    YELLOW = "\033[33m"
    RESET = "\033[0m"


_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def strip_ansi(text: str) -> str:
    return _ANSI_RE.sub("", text)


def paint(text: str, code: str, enabled: bool) -> str:
    return f"{code}{text}{Style.RESET}" if enabled else text


def format_duration(seconds: float) -> str:
    if seconds < 60:
        return f"{seconds:.1f}s"
    minutes, remainder = divmod(int(seconds), 60)
    return f"{minutes}m {remainder:02d}s"


def shorten(text: str, width: int, ellipsis: str) -> str:
    if width <= 0 or len(text) <= width:
        return text
    if width <= len(ellipsis):
        return text[:width]
    return text[: width - len(ellipsis)] + ellipsis


def shorten_middle(text: str, width: int, ellipsis: str) -> str:
    """Keep both ends so a long Cargo feature list stays readable."""
    if width <= 0 or len(text) <= width:
        return text
    if width <= len(ellipsis):
        return text[:width]
    available = width - len(ellipsis)
    left = (available + 1) // 2
    right = available - left
    return text[:left] + ellipsis + text[-right:]


def split_targets(values: Iterable[str]) -> list[str]:
    targets: list[str] = []
    for value in values:
        for part in value.split(","):
            part = part.strip()
            if part:
                targets.append(part)
    return targets


def make_symbols() -> dict[str, str]:
    encoding = (sys.stdout.encoding or "").lower()
    if "utf" in encoding:
        return {
            "pass": "✓",
            "fail": "✗",
            "skip": "–",
            "run": "⏺",
            "ellipsis": "…",
            "dot": "·",
            "bar": "─",
        }
    return {
        "pass": "PASS",
        "fail": "FAIL",
        "skip": "SKIP",
        "run": "RUN",
        "ellipsis": "...",
        "dot": ".",
        "bar": "-",
    }


@dataclass(frozen=True)
class Suite:
    name: str
    area: str
    label: str
    command: tuple[str, ...]
    prerequisite: bool = False


def build_suites() -> list[Suite]:
    return [
        Suite(
            "rust/fmt",
            "rust",
            "format",
            ("cargo", "fmt", "--all", "--", "--check"),
        ),
        Suite(
            "rust/check",
            "rust",
            "workspace check",
            ("cargo", "check", "--workspace"),
        ),
        Suite("rust/core", "rust", "core", ("cargo", "test", "-p", "fft_core")),
        Suite(
            "rust/runtime",
            "rust",
            "runtime",
            ("cargo", "test", "-p", "fft_runtime"),
        ),
        Suite("rust/app", "rust", "app", ("cargo", "test", "-p", "fft_app")),
        Suite(
            "rust/app-host",
            "rust",
            "app host",
            ("cargo", "test", "-p", "fft_app_host"),
        ),
        Suite(
            "rust/test-support",
            "rust",
            "test support",
            ("cargo", "test", "-p", "fft_test"),
        ),
        Suite(
            "rust/script",
            "rust",
            "script",
            ("cargo", "test", "-p", "fft_script"),
        ),
        Suite(
            "rust/wasm",
            "rust",
            "wasm tests",
            ("cargo", "test", "-p", "fft_wasm"),
        ),
        Suite(
            "rust/shipping",
            "rust",
            "shipping tests",
            ("cargo", "test", "-p", "fft_shipping"),
        ),
        Suite(
            "rust/http",
            "rust",
            "http adapter",
            ("cargo", "test", "-p", "fft_http_adapter"),
        ),
        # Suite(
        #     "rust/tauri",
        #     "rust",
        #     "tauri adapter",
        #     ("cargo", "test", "-p", "fft_tauri_adapter"),
        # ),
        Suite(
            "rust/tauri-python",
            "rust",
            "tauri + python",
            ("cargo", "test", "-p", "fft_tauri_adapter", "--features", "python"),
        ),
        Suite(
            "rust/python",
            "rust",
            "python adapter",
            ("cargo", "test", "-p", "fft_python_adapter", "--features", "python"),
        ),
        # Suite(
        #     "rust/desktop",
        #     "rust",
        #     "desktop · inert",
        #     ("cargo", "test", "-p", "fft_desktop", "--no-default-features"),
        # ),
        # Suite(
        #     "rust/desktop-http",
        #     "rust",
        #     "desktop · http",
        #     (
        #         "cargo",
        #         "test",
        #         "-p",
        #         "fft_desktop",
        #         "--no-default-features",
        #         "--features",
        #         "http",
        #     ),
        # ),
        # Suite(
        #     "rust/desktop-python",
        #     "rust",
        #     "desktop · python",
        #     (
        #         "cargo",
        #         "test",
        #         "-p",
        #         "fft_desktop",
        #         "--no-default-features",
        #         "--features",
        #         "python",
        #     ),
        # ),
        Suite(
            "rust/desktop-http-python",
            "rust",
            "desktop · http+python",
            (
                "cargo",
                "test",
                "-p",
                "fft_desktop",
                "--no-default-features",
                "--features",
                "http,python",
            ),
        ),
        # Suite(
        #     "rust/desktop-tauri",
        #     "rust",
        #     "desktop · tauri",
        #     (
        #         "cargo",
        #         "test",
        #         "-p",
        #         "fft_desktop",
        #         "--no-default-features",
        #         "--features",
        #         "tauri",
        #     ),
        # ),
        # Suite(
        #     "rust/desktop-tauri-python",
        #     "rust",
        #     "desktop · tauri+python",
        #     (
        #         "cargo",
        #         "test",
        #         "-p",
        #         "fft_desktop",
        #         "--no-default-features",
        #         "--features",
        #         "tauri_python",
        #     ),
        # ),
        # Suite(
        #     "rust/check-tauri",
        #     "rust",
        #     "tauri check",
        #     (
        #         "cargo",
        #         "check",
        #         "-p",
        #         "fft_desktop",
        #         "--no-default-features",
        #         "--features",
        #         "tauri",
        #     ),
        # ),
        Suite(
            "rust/check-tauri-python",
            "rust",
            "tauri+python check",
            (
                "cargo",
                "check",
                "-p",
                "fft_desktop",
                "--no-default-features",
                "--features",
                "tauri_python,http,python",
            ),
        ),
        Suite(
            "frontend/typecheck",
            "frontend",
            "typecheck",
            ("npm", "--prefix", "web", "run", "typecheck"),
        ),
        Suite(
            "frontend/unit",
            "frontend",
            "unit",
            ("npm", "--prefix", "web", "test"),
        ),
        Suite(
            "frontend/build",
            "frontend",
            "build",
            ("npm", "--prefix", "web", "run", "build"),
        ),
        Suite(
            "e2e/playwright",
            "e2e",
            "playwright",
            ("npm", "--prefix", "web", "run", "test:e2e"),
        ),
        Suite(
            "wasm/check",
            "wasm",
            "wasm check",
            ("cargo", "check", "-p", "fft_wasm", "--target", "wasm32-unknown-unknown"),
        ),
    ]


def wasm_target_installed() -> bool:
    rustup = shutil.which("rustup")
    if not rustup:
        return False
    try:
        result = subprocess.run(
            [rustup, "target", "list", "--installed"],
            capture_output=True,
            text=True,
            timeout=15,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return (
        result.returncode == 0
        and "wasm32-unknown-unknown" in result.stdout.split()
    )


class UsageError(Exception):
    pass


def _suggestion(name: str, choices: Sequence[str]) -> str:
    matches = difflib.get_close_matches(name, choices, n=3, cutoff=0.45)
    if not matches:
        return ""
    return f" Did you mean: {', '.join(matches)}?"


def expand_targets(targets: Sequence[str], suites: Sequence[Suite]) -> list[str]:
    by_name = {suite.name: suite for suite in suites}
    by_area = {
        area: [suite.name for suite in suites if suite.area == area]
        for area in AREA_ORDER
    }
    names: list[str] = []
    for target in targets:
        if target == "all":
            names.extend(suite.name for suite in suites)
        elif target in by_area:
            names.extend(by_area[target])
        elif target in by_name:
            names.append(target)
        else:
            choices = ["all", *AREA_ORDER, *by_name]
            raise UsageError(
                f"unknown target {target!r}.{_suggestion(target, choices)} "
                "Try --list."
            )
    wanted = set(names)
    return [suite.name for suite in suites if suite.name in wanted]


def resolve_selection(args: argparse.Namespace, suites: Sequence[Suite]) -> list[Suite]:
    targets = split_targets(args.targets) or ["all"]
    skip_targets = split_targets(args.skip)
    if args.fast:
        skip_targets.extend(["e2e", "wasm"])

    selected = expand_targets(targets, suites)
    skipped = set(expand_targets(skip_targets, suites)) if skip_targets else set()
    names = [name for name in selected if name not in skipped]
    if not names:
        raise UsageError("nothing to run after applying --skip/--fast.")

    prerequisites: set[str] = set()
    if "e2e/playwright" in names:
        if "frontend/build" in skipped:
            raise UsageError(
                "e2e/playwright needs frontend/build; drop "
                "--skip frontend/build."
            )
        if "frontend/build" not in names:
            names.append("frontend/build")
            prerequisites.add("frontend/build")

    wanted = set(names)
    return [
        replace(suite, prerequisite=suite.name in prerequisites)
        for suite in suites
        if suite.name in wanted
    ]


def print_list(suites: Sequence[Suite], no_color: bool = False) -> None:
    color = (
        not no_color
        and sys.stdout.isatty()
        and "NO_COLOR" not in os.environ
    )
    print(paint("fft_audio test suites", Style.BOLD, color))
    print()
    for area in AREA_ORDER:
        print(paint(AREA_LABELS[area].upper(), Style.BOLD, color))
        print(f"  {AREA_DESCRIPTIONS[area]}")
        for suite in suites:
            if suite.area != area:
                continue
            command = shlex.join(suite.command)
            print(f"  {suite.name:<26} {paint(command, Style.DIM, color)}")
        print()
    print("Areas:   " + ", ".join(AREA_ORDER))
    print("Suites:  " + ", ".join(suite.name for suite in suites))
    print("Default: all areas; e2e automatically builds web/dist first.")
    if not wasm_target_installed():
        print(
            "Note:    the wasm32 target is not installed; wasm/check is skipped "
            "in a default run."
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="scripts/dev.py",
        description=(
            "Run the fft_audio Rust, frontend, and browser test suites from "
            "one command."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "examples:\n"
            "  python3 scripts/dev.py                    # everything\n"
            "  python3 scripts/dev.py frontend           # typecheck + unit + build\n"
            "  python3 scripts/dev.py rust -v            # Rust matrix, full output\n"
            "  python3 scripts/dev.py frontend e2e       # UI change with browser tests\n"
            "  python3 scripts/dev.py rust/core          # one suite\n"
            "  python3 scripts/dev.py --skip e2e         # everything except Playwright\n"
            "  python3 scripts/dev.py --list\n"
            "\n"
            "prerequisites:\n"
            "  Rust/Cargo and Node/npm; run `npm --prefix web install` once before\n"
            "  frontend or e2e checks. Playwright needs its browser installed.\n"
        ),
    )
    parser.add_argument(
        "targets",
        nargs="*",
        metavar="target",
        help=(
            "areas (rust, frontend, e2e, wasm) or suite names; default: all. "
            "Comma-separated values are accepted."
        ),
    )
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="stream full command output instead of a failure excerpt",
    )
    parser.add_argument(
        "-l",
        "--list",
        action="store_true",
        help="list every area and suite with its command, then exit",
    )
    parser.add_argument(
        "--skip",
        action="append",
        default=[],
        metavar="target",
        help="skip an area or suite (repeatable; accepts comma-separated values)",
    )
    parser.add_argument(
        "--fast",
        action="store_true",
        help="skip e2e and wasm (same as --skip e2e --skip wasm)",
    )
    parser.add_argument(
        "--no-color",
        action="store_true",
        help="disable ANSI color",
    )
    parser.add_argument(
        "--fail-fast",
        action="store_true",
        help="stop after the first failing suite",
    )
    return parser


@dataclass
class RunResult:
    suite: Suite
    status: str
    duration: float
    output: str = ""
    reason: str | None = None
    count: int | None = None


def extract_test_count(suite: Suite, output: str) -> int | None:
    clean = strip_ansi(output)
    if suite.name == "frontend/unit":
        matches = re.findall(r"\bTests\s+(\d+)\s+passed\b", clean)
        return int(matches[-1]) if matches else None
    if suite.name == "e2e/playwright":
        matches = re.findall(r"(\d+)\s+passed\b", clean)
        return int(matches[-1]) if matches else None
    if (
        suite.area == "rust"
        and len(suite.command) >= 2
        and suite.command[0] == "cargo"
        and suite.command[1] == "test"
    ):
        matches = re.findall(r"test result: ok\. (\d+) passed", clean)
        return sum(int(value) for value in matches) if matches else None
    return None


class Runner:
    def __init__(self, args: argparse.Namespace, suites: Sequence[Suite]) -> None:
        self.args = args
        self.suites = list(suites)
        self.symbols = make_symbols()
        self.color = (
            not args.no_color
            and sys.stdout.isatty()
            and "NO_COLOR" not in os.environ
        )
        self.tty = sys.stdout.isatty()
        self.verbose = args.verbose
        self.term_width = shutil.get_terminal_size((120, 20)).columns
        self.label_width = min(
            26, max(len(self._label(suite)) for suite in self.suites)
        )
        self.show_command = self.term_width >= 96
        self.cmd_width = (
            min(64, max(24, self.term_width - self.label_width - 16))
            if self.show_command
            else 0
        )
        self.results: list[RunResult] = []
        self.start_time = time.perf_counter()
        self.wasm_available = (
            wasm_target_installed()
            if any(suite.name == "wasm/check" for suite in self.suites)
            else True
        )
        explicit = split_targets(args.targets)
        self.wasm_explicit = any(
            value in ("wasm", "wasm/check") for value in explicit
        )

    def _label(self, suite: Suite) -> str:
        label = suite.label.replace("·", self.symbols["dot"])
        return label + (" (prereq)" if suite.prerequisite else "")

    def _paint(self, text: str, code: str) -> str:
        return paint(text, code, self.color)

    def _duration(self, seconds: float) -> str:
        return format_duration(seconds)

    def _command_text(self, suite: Suite) -> str:
        return shlex.join(suite.command)

    def _result_command(self, suite: Suite) -> str:
        return shorten_middle(
            self._command_text(suite), self.cmd_width, self.symbols["ellipsis"]
        )

    def _print_header(self) -> None:
        branch = ""
        try:
            git = subprocess.run(
                ["git", "rev-parse", "--abbrev-ref", "HEAD"],
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=5,
            )
            if git.returncode == 0:
                branch = git.stdout.strip()
        except (OSError, subprocess.TimeoutExpired):
            pass
        stamp = time.strftime("%Y-%m-%d %H:%M:%S")
        title = f" {self.symbols['dot']} ".join(
            part for part in ("fft_audio dev checks", branch, stamp) if part
        )
        print(self._paint(title, Style.BOLD))

    def _preflight(self, suite: Suite) -> tuple[str, str] | None:
        if suite.area in ("rust", "wasm") and shutil.which("cargo") is None:
            return ("fail", "cargo not found on PATH")
        if suite.area in ("frontend", "e2e"):
            if shutil.which("npm") is None:
                return ("fail", "npm not found on PATH")
            if not (WEB / "node_modules").is_dir():
                return (
                    "fail",
                    "web/node_modules is missing; run `npm --prefix web install`",
                )
        if suite.name == "wasm/check" and not self.wasm_available:
            if self.wasm_explicit:
                return (
                    "fail",
                    "wasm32 target not installed; run "
                    "`rustup target add wasm32-unknown-unknown`",
                )
            return ("skip", "wasm32 target not installed")
        return None

    def _environment(self, suite: Suite) -> dict[str, str]:
        env = os.environ.copy()
        if suite.area in ("rust", "wasm"):
            env["FFT_SKIP_WEB_BUILD"] = "1"
            env["CARGO_TERM_COLOR"] = (
                "always" if self.color and self.verbose else "never"
            )
        if not self.color:
            env.setdefault("NO_COLOR", "1")
        return env

    def _draw_running(self, suite: Suite, elapsed: float) -> None:
        label = f"{self._label(suite):<{self.label_width}}"
        if self.show_command:
            command = f"{self._result_command(suite):<{self.cmd_width}}  "
        else:
            command = ""
        line = (
            f"  {self.symbols['run']} {label}  {command}"
            f"{self._duration(elapsed):>7}"
        )
        sys.stdout.write("\r\033[K" + self._paint(line, Style.DIM))
        sys.stdout.flush()

    def _clear_running(self) -> None:
        if self.tty and not self.verbose:
            sys.stdout.write("\r\033[K")
            sys.stdout.flush()

    def _stop_process(self, process: subprocess.Popen[str]) -> None:
        if process.poll() is not None:
            return
        if os.name == "posix":
            try:
                os.killpg(os.getpgid(process.pid), signal.SIGINT)
            except (ProcessLookupError, PermissionError):
                pass
        else:
            process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            if os.name == "posix":
                try:
                    os.killpg(os.getpgid(process.pid), signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
            else:
                process.kill()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass

    def _run_suite(self, suite: Suite) -> RunResult:
        preflight = self._preflight(suite)
        if preflight is not None:
            status, reason = preflight
            return RunResult(suite, status, 0.0, reason=reason)

        command = list(suite.command)
        executable = shutil.which(command[0])
        if executable:
            command[0] = executable
        start = time.perf_counter()
        if self.verbose:
            print(
                self._paint(
                    f"[{suite.name}] $ {self._command_text(suite)}",
                    Style.DIM,
                )
            )
            sys.stdout.flush()
        elif not self.tty:
            print(f"RUN  {suite.name}")
            sys.stdout.flush()

        try:
            process = subprocess.Popen(
                command,
                cwd=ROOT,
                env=self._environment(suite),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                **({"start_new_session": True} if os.name == "posix" else {}),
            )
        except OSError as error:
            return RunResult(
                suite,
                "fail",
                time.perf_counter() - start,
                reason=f"could not start command: {error}",
            )

        sink: collections.deque[str] = collections.deque(maxlen=CAPTURE_LINES)
        write_lock = threading.Lock()

        def pump() -> None:
            if process.stdout is None:
                return
            try:
                for line in process.stdout:
                    sink.append(line)
                    if self.verbose:
                        with write_lock:
                            sys.stdout.write(line)
                            sys.stdout.flush()
            finally:
                process.stdout.close()

        reader = threading.Thread(target=pump, daemon=True)
        reader.start()
        try:
            while process.poll() is None:
                if self.tty and not self.verbose:
                    self._draw_running(suite, time.perf_counter() - start)
                time.sleep(0.1)
            returncode = process.wait()
        except KeyboardInterrupt:
            self._stop_process(process)
            raise
        finally:
            if process.poll() is None:
                self._stop_process(process)
            reader.join(timeout=2.0)
            self._clear_running()

        duration = time.perf_counter() - start
        output = "".join(sink)
        status = "pass" if returncode == 0 else "fail"
        count = extract_test_count(suite, output) if status == "pass" else None
        return RunResult(suite, status, duration, output=output, count=count)

    def _print_failure_detail(self, result: RunResult) -> None:
        if result.reason:
            print(f"      {result.reason}")
        lines = [
            strip_ansi(line).rstrip()
            for line in result.output.splitlines()
        ]
        while lines and not lines[-1]:
            lines.pop()
        if not lines:
            return
        hidden = max(0, len(lines) - EXCERPT_LINES)
        for line in lines[-EXCERPT_LINES:]:
            line = shorten(line, EXCERPT_WIDTH, self.symbols["ellipsis"])
            print(f"      {line}")
        if hidden:
            print(
                f"      {self.symbols['ellipsis']} {hidden} earlier line(s); "
                "re-run with -v for full output"
            )

    def _print_result(self, result: RunResult) -> None:
        suite = result.suite
        if self.verbose:
            if result.status == "pass":
                count = (
                    f" {result.count} passed"
                    if result.count is not None
                    else ""
                )
                print(
                    f"[{suite.name}] "
                    f"{self._paint('PASS', Style.GREEN)} "
                    f"{self._duration(result.duration)}{count}"
                )
            elif result.status == "skip":
                print(
                    f"[{suite.name}] "
                    f"{self._paint('SKIP', Style.YELLOW)} "
                    f"{result.reason}"
                )
            else:
                print(
                    f"[{suite.name}] "
                    f"{self._paint('FAIL', Style.RED)} "
                    f"{self._duration(result.duration)}"
                )
                if result.reason:
                    print(f"      {result.reason}")
            return

        if result.status == "pass":
            symbol = self._paint(self.symbols["pass"], Style.GREEN)
            duration = self._duration(result.duration)
        elif result.status == "skip":
            symbol = self._paint(self.symbols["skip"], Style.YELLOW)
            duration = "skipped"
        else:
            symbol = self._paint(self.symbols["fail"], Style.RED)
            duration = self._duration(result.duration)

        label = f"{self._label(suite):<{self.label_width}}"
        if self.show_command:
            command = f"{self._result_command(suite):<{self.cmd_width}}"
            prefix = f"  {symbol} {label}  {self._paint(command, Style.DIM)}  "
        else:
            prefix = f"  {symbol} {label}  "
        count = f"  {result.count} passed" if result.count is not None else ""
        print(f"{prefix}{duration:>7}{count}")
        if result.status == "fail":
            self._print_failure_detail(result)

    def _print_summary(self) -> None:
        elapsed = time.perf_counter() - self.start_time
        passed = sum(result.status == "pass" for result in self.results)
        failed = sum(result.status == "fail" for result in self.results)
        skipped = sum(result.status == "skip" for result in self.results)
        print()
        print(self._paint(self.symbols["bar"] * SEPARATOR_WIDTH, Style.DIM))
        total_color = Style.GREEN if failed == 0 else Style.RED
        total = f" {self.symbols['dot']} ".join(
            [
                f"{passed} passed",
                f"{failed} failed",
                f"{skipped} skipped",
                self._duration(elapsed),
            ]
        )
        print(self._paint(total, total_color))

        parts: list[str] = []
        for area in AREA_ORDER:
            results = [
                result for result in self.results if result.suite.area == area
            ]
            if not results:
                continue
            area_passed = sum(result.status == "pass" for result in results)
            area_failed = sum(result.status == "fail" for result in results)
            area_skipped = sum(result.status == "skip" for result in results)
            if area_passed == 0 and area_failed == 0 and area_skipped > 0:
                text = f"{AREA_LABELS[area]} skipped"
                color = Style.YELLOW
            else:
                text = f"{AREA_LABELS[area]} {area_passed}/{len(results)}"
                color = Style.GREEN if area_failed == 0 else Style.RED
            parts.append(self._paint(text, color))
        if parts:
            print("   ".join(parts))
        if self.args.fail_fast and failed:
            print(self._paint("stopped at first failure", Style.DIM))

    def run(self) -> int:
        self._print_header()
        current_area: str | None = None
        for suite in self.suites:
            if suite.area != current_area:
                if current_area is not None:
                    print()
                print(self._paint(AREA_LABELS[suite.area].upper(), Style.BOLD))
                current_area = suite.area
            result = self._run_suite(suite)
            self.results.append(result)
            self._print_result(result)
            if result.status == "fail" and self.args.fail_fast:
                break
        self._print_summary()
        return 1 if any(result.status == "fail" for result in self.results) else 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    suites = build_suites()

    if args.list:
        print_list(suites, args.no_color)
        return 0

    try:
        selected = resolve_selection(args, suites)
    except UsageError as error:
        parser.error(str(error))

    runner = Runner(args, selected)
    try:
        return runner.run()
    except KeyboardInterrupt:
        print("\nInterrupted.")
        return 130


if __name__ == "__main__":
    sys.exit(main())
