"""Finds the built civ6opt and runs it, the same way on Windows, macOS and Linux.

The other tools import this module; it is not run by itself.
"""
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def binary():
    """The executable `cargo build --release` makes in optimizer/ for the machine it runs on."""
    exe = ROOT / "optimizer" / "target" / "release" / ("civ6opt.exe" if os.name == "nt" else "civ6opt")
    if not exe.exists():
        sys.exit(f"{exe} is missing: run `cargo build --release` in optimizer/")
    return exe


def low_priority():
    """Keyword arguments of subprocess that start a process at the lowest scheduling priority, so a long search
    leaves the machine usable."""
    if os.name == "nt":
        return {"creationflags": subprocess.IDLE_PRIORITY_CLASS}
    return {"preexec_fn": lambda: os.nice(19)}


def run(args, **kw):
    """Runs civ6opt with `args` (strings or paths) at the lowest priority; `kw` goes to subprocess.run."""
    return subprocess.run([str(binary()), *map(str, args)], **low_priority(), **kw)
