from __future__ import annotations

import sys

import torch


def training_device(backend: str = "auto", adapter: int = 0) -> tuple[torch.device, str]:
    if backend == "auto":
        backend = "cuda" if torch.cuda.is_available() else "directml" if sys.platform == "win32" else "cpu"
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
    if backend == "cpu":
        return torch.device("cpu"), "CPU"
    raise ValueError(f"Unknown backend: {backend}")
