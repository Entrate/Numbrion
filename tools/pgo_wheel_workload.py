"""Run the PGO workload (crates/pyengine/src/pgo.rs) inside an instrumented numbrion wheel.

    python tools/pgo_wheel_workload.py <wheel dir> <extract dir> <seconds> <team pool>...

Used by tools/pgo.sh and tools/pgo.ps1. Unpacks the single wheel in <wheel dir> into <extract dir> and loads
the extension module straight from there: nothing is installed and numpy is not needed, so any Python >= 3.9
works. The profile is written when the interpreter exits.
"""

import glob
import importlib.util
import os
import shutil
import sys
import zipfile


def main() -> None:
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    wheel_dir, dest, seconds, pools = sys.argv[1], sys.argv[2], float(sys.argv[3]), sys.argv[4:]
    wheels = glob.glob(os.path.join(wheel_dir, "*.whl"))
    if len(wheels) != 1:
        sys.exit(f"pgo_wheel_workload: expected one wheel in {wheel_dir}, found {len(wheels)}")
    shutil.rmtree(dest, ignore_errors=True)
    with zipfile.ZipFile(wheels[0]) as z:
        z.extractall(dest)
    ext = [p for p in glob.glob(os.path.join(dest, "numbrion", "_numbrion*")) if p.endswith((".so", ".pyd"))]
    if len(ext) != 1:
        sys.exit(f"pgo_wheel_workload: no extension module in {wheels[0]}")
    spec = importlib.util.spec_from_file_location("_numbrion", ext[0])
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    print(module._pgo_workload([os.path.abspath(p) for p in pools], seconds), end="", flush=True)


if __name__ == "__main__":
    main()
