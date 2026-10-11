"""Compare two raw array directories (arrays.py), e.g. the Python reference and a Rust parity example's output.

Usage: python compare.py REFERENCE_DIR RUST_DIR [NAME ...] [--atol 1e-5] [--rtol 1e-5]
Exit status 1 when any compared array differs.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import arrays  # noqa: E402


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("reference", type=Path)
    parser.add_argument("rust", type=Path)
    parser.add_argument("names", nargs="*")
    parser.add_argument("--atol", type=float, default=1e-5)
    parser.add_argument("--rtol", type=float, default=1e-5)
    parser.add_argument("--show", type=int, default=5)
    args = parser.parse_args()
    ref, out = arrays.load(args.reference), arrays.load(args.rust)
    names = args.names or [n for n in ref if n in out]
    ok = all([arrays.compare(n, ref[n], out[n], args.atol, args.rtol, args.show) for n in names])
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
