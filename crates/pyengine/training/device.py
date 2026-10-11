from __future__ import annotations

import platform
import subprocess
import sys

import torch

BACKENDS = ["auto", "directml", "cuda", "mps", "cpu"]


def training_device(backend: str = "auto", adapter: int = 0) -> tuple[torch.device, str]:
    if backend == "auto":
        if torch.cuda.is_available():
            backend = "cuda"
        elif sys.platform == "win32":
            backend = "directml"
        elif torch.backends.mps.is_available():
            backend = "mps"
        else:
            backend = "cpu"
    if backend == "directml":
        import torch_directml

        count = torch_directml.device_count()
        if not 0 <= adapter < count:
            raise ValueError(f"DirectML adapter {adapter} unavailable; detected {count} adapters")
        return torch_directml.device(adapter), torch_directml.device_name(adapter).rstrip("\x00")
    if backend == "cuda":
        if not torch.cuda.is_available():
            raise RuntimeError("CUDA is unavailable; AMD GPUs on Windows use --device directml")
        return torch.device(f"cuda:{adapter}"), torch.cuda.get_device_name(adapter)
    if backend == "mps":
        if not torch.backends.mps.is_available():
            reason = "this torch build lacks MPS" if not torch.backends.mps.is_built() else "no Metal GPU (macOS 12.3+ on Apple silicon)"
            raise RuntimeError(f"MPS is unavailable: {reason}")
        # Tensors report mps:0, and torch.device("mps") != torch.device("mps:0").
        return torch.device("mps:0"), f"Apple GPU ({_apple_chip()})"
    if backend == "cpu":
        return torch.device("cpu"), "CPU"
    raise ValueError(f"Unknown backend: {backend}")


def _apple_chip() -> str:
    try:
        return subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True,
                              check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return platform.machine()
