"""Optional crash-readable battle diagnostics. Disk work never runs on the decision thread."""
from __future__ import annotations

import json
import logging
import math
import queue
import threading
import time
from pathlib import Path

from play.runstore import safe_id

log = logging.getLogger("play")


def serializable(value):
    """Handle numpy arrays/scalars without requiring numpy for ordinary play bots."""
    if isinstance(value, dict):
        return {str(k): serializable(v) for k, v in value.items()}
    if isinstance(value, (tuple, list)):
        return [serializable(v) for v in value]
    if hasattr(value, "tolist"):
        return serializable(value.tolist())
    if isinstance(value, float):
        return round(value, 6) if math.isfinite(value) else None
    return value


class BattleRecorder:
    """Bounded, nonblocking queue; errors/drop counts are logged, never propagated to play.

    Each line is flushed before the next event. Append mode preserves reconnects and
    supervisor resumes. The normal RunStore remains the authoritative raw log.
    """

    def __init__(self, directory: Path, clean=lambda v: v):
        self.directory, self.clean = Path(directory), clean
        self.queue = queue.Queue(maxsize=512)
        self.dropped = 0
        self.closed = False
        self.thread = threading.Thread(target=self._run, name="battle-diagnostics", daemon=True)
        self.thread.start()

    def emit(self, room: str, kind: str, data: dict):
        if self.closed:
            return
        try:
            safe_id(room)
            self.queue.put_nowait({**data, "room": room, "kind": kind,
                                   "t": data.get("time", time.time())})
        except Exception:
            self.dropped += 1
            log.exception("Battle diagnostic enqueue failed; dropped=%s", self.dropped)

    def close(self):
        self.closed = True
        # A blocked disk must not prevent shutdown either.
        self.thread.join(timeout=2)
        if self.thread.is_alive():
            log.error("Battle diagnostic writer did not drain within 2 seconds")

    def _run(self):
        while not self.closed or not self.queue.empty():
            try:
                event = self.queue.get(timeout=.05)
            except queue.Empty:
                continue
            try:
                self.directory.mkdir(parents=True, exist_ok=True)
                payload = self.clean(serializable(event))
                with (self.directory / (event["room"] + ".jsonl")).open("a", encoding="utf-8", newline="\n") as handle:
                    handle.write(json.dumps(payload, ensure_ascii=False, separators=(",", ":"), allow_nan=False) + "\n")
                    if event["kind"] == "message":
                        for line in event.get("text", "").splitlines():
                            if line.startswith("|request|"):
                                request = json.loads(line[len("|request|"):])
                                handle.write(json.dumps({"t": event["t"], "room": event["room"],
                                                         "kind": "request_received", "request": request},
                                                        separators=(",", ":")) + "\n")
                    handle.flush()
            except Exception:
                log.exception("Battle diagnostic write failed for %s", event["room"])
            finally:
                self.queue.task_done()
