"""Raw array directories shared by the Python reference dumps and the Rust parity examples.

A directory holds ``meta.json`` ({name: [dtype, shape]}) and one ``<name>.bin`` per array: C-order, little-endian
raw bytes (dtypes int64, int32, float32, bool). ``examples/common/mod.rs`` reads and writes the same format.
"""
from __future__ import annotations

import json
from pathlib import Path

import numpy as np

DTYPES = {"int64": np.int64, "int32": np.int32, "float32": np.float32, "bool": np.bool_}


def save(directory: Path, arrays: dict) -> None:
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    meta = {}
    for name, array in arrays.items():
        array = np.ascontiguousarray(array)
        dtype = next(k for k, v in DTYPES.items() if array.dtype == v)
        meta[name] = [dtype, list(array.shape)]
        array.astype(array.dtype.newbyteorder("<"), copy=False).tofile(directory / f"{name}.bin")
    (directory / "meta.json").write_text(json.dumps(meta))


def load(directory: Path) -> dict:
    directory = Path(directory)
    meta = json.loads((directory / "meta.json").read_text())
    return {name: np.fromfile(directory / f"{name}.bin", dtype=np.dtype(DTYPES[dtype]).newbyteorder("<")).reshape(shape)
            for name, (dtype, shape) in meta.items()}


def compare(name: str, ref: np.ndarray, out: np.ndarray, atol: float = 1e-5, rtol: float = 1e-5, show: int = 5) -> bool:
    """Print a summary of the mismatches of one array; True when they agree."""
    if ref.shape != out.shape:
        print(f"{name}: shape {out.shape} != reference {ref.shape}")
        return False
    if ref.dtype.kind == "f":
        bad = ~np.isclose(out, ref, atol=atol, rtol=rtol, equal_nan=True)
    else:
        bad = out != ref
    n = int(bad.sum())
    if not n:
        print(f"{name}: ok ({ref.size} values)")
        return True
    print(f"{name}: {n} of {ref.size} differ")
    for index in np.argwhere(bad)[:show]:
        index = tuple(int(i) for i in index)
        print(f"  {index}: rust {out[index]!r} python {ref[index]!r}")
    return False
