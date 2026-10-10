"""Verify forward, backward, and optimizer updates on the selected training device."""
from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import torch

from training.device import training_device


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--device", choices=["auto", "directml", "cuda", "cpu"], default="auto")
    parser.add_argument("--adapter", type=int, default=0)
    parser.add_argument("--steps", type=int, default=50)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.steps < 1:
        parser.error("--steps must be positive")
    torch.set_num_threads(2)
    torch.manual_seed(7)
    device, name = training_device(args.device, args.adapter)
    model = torch.nn.Sequential(torch.nn.Linear(128, 128), torch.nn.Tanh(), torch.nn.Linear(128, 1)).to(device)
    optimizer = torch.optim.Adam(model.parameters(), lr=0.01, foreach=False)
    x = torch.randn(256, 128).to(device)
    y = (x[:, :4].sum(dim=1) * 0.25).reshape(-1, 1)
    before = [p.detach().cpu().clone() for p in model.parameters()]
    start = time.perf_counter()
    losses = []
    for _ in range(args.steps):
        optimizer.zero_grad(set_to_none=True)
        loss = (model(x) - y).square().mean()
        loss.backward()
        if not all(p.grad is not None and p.grad.device == device for p in model.parameters()):
            raise RuntimeError("A gradient is missing or on the wrong device")
        optimizer.step()
        losses.append(float(loss.detach().cpu()))
    changed = any(not torch.equal(old, new.detach().cpu()) for old, new in zip(before, model.parameters()))
    if not changed or not losses[-1] < losses[0]:
        raise RuntimeError(f"GPU training check failed: changed={changed}, losses={losses[0], losses[-1]}")
    report = {
        "adapter": name,
        "device": str(device),
        "torch": torch.__version__,
        "steps": args.steps,
        "seconds": time.perf_counter() - start,
        "initial_loss": losses[0],
        "final_loss": losses[-1],
        "parameters_changed": changed,
    }
    print(json.dumps(report, indent=2))
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
