"""Python reference outputs for the parity checks: runs training.encoder.Encoder (the Python implementation) over an
observation JSONL from observations.py and saves every array of every boundary, stacked as [boundaries, rows, ...],
to a raw array directory (arrays.py).

Usage: python reference.py OBS.jsonl OUT_DIR
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / "pyengine"))

import numpy as np  # noqa: E402

import arrays  # noqa: E402
from training import encoder as py_encoder  # noqa: E402

KEYS = ("ids", "floats", "field", "foe_match", "fainted", "guard", "action_features", "token_features")


def main():
    obs_path, out = Path(sys.argv[1]), Path(sys.argv[2])
    Encoder = getattr(py_encoder, "PyEncoder", py_encoder.Encoder)  # the Python implementation
    lines = obs_path.read_text(encoding="utf-8").splitlines()
    rows = len(json.loads(lines[0])["observations"])
    encoder = Encoder(rows)
    stacked = {k: [] for k in KEYS}
    started = time.perf_counter()
    for line in lines:
        b = encoder.encode_batch(json.loads(line)["observations"])
        for k in KEYS:
            stacked[k].append(getattr(b, k).copy())
    seconds = time.perf_counter() - started
    arrays.save(out, {k: np.stack(v) for k, v in stacked.items()})
    print(f"{len(lines)} boundaries x {rows} rows in {seconds:.2f}s ({1e3 * seconds / len(lines) / rows:.3f} ms/row) -> {out}")


if __name__ == "__main__":
    main()
