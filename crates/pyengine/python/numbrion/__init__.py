"""Training interface for the numbrion ``gen9randomdoublesbattle`` engine.

``Battle`` is one battle (debugging, scripted play, search); ``BatchEnv`` runs many in parallel with
auto-reset; ``GridExecutor`` runs one-decision search grids in parallel. See ``docs/design/TRAINING-API.md``
for the action encoding, masks, observation rules and the search API.
"""

import os
import warnings

from ._numbrion import (  # noqa: F401
    FORCED_MOVE,
    N_ACTIONS,
    N_MOVE_ACTIONS,
    PASS,
    SWITCH_BASE,
    TARGET_AUTO,
    BatchEnv,
    Battle,
    ChoiceError,
    __version__,
    decode_action,
    describe_action,
    move_action,
    switch_action,
)
from . import _numbrion as _native

ENCODER = getattr(_native, "ENCODER", False)
if ENCODER:
    from ._numbrion import ObsEncoder, SmartAgent  # noqa: F401
from ._numbrion import SEARCH  # noqa: F401  (built with the opt-in `search` feature)
from ._numbrion import GridExecutor as _GridExecutor
from .utils import grid_actions, sample_masked, sample_uniform  # noqa: F401

if SEARCH:
    from ._numbrion import RevealTracker, SetPool, WorldBuilder  # noqa: F401

_SEARCH_PREREQUISITES = (
    "numbrion.GridExecutor: search prerequisites are not all done - see docs/design/SEARCH-PREREQUISITES.md. "
    "Search roots must be fair-information worlds (numbrion.WorldBuilder, wheel built with --features python,search), "
    "and training wheels should be PGO builds. Set NUMBRION_SEARCH_PREREQS_DONE=1 once both are done."
)

_LIVE_ROOTS = (
    "numbrion.GridExecutor.run: some roots are not WorldBuilder worlds (Battle.is_world is False). A live battle "
    "or its clone carries the opponent's true hidden information; use worlds for training targets and strength "
    "claims, or pass allow_live=True for debugging and benchmarks."
)


class GridExecutor:
    """``numbrion._numbrion.GridExecutor`` plus a check that search roots are fair-information worlds.

    In search wheels (``numbrion.SEARCH``), ``run(roots, actions, ..., allow_live=False)`` warns when a root
    is not a ``WorldBuilder`` world (``Battle.is_world``); everything else is passed through unchanged.
    """

    def __init__(self, threads=0):
        self._ex = _GridExecutor(threads)

    @property
    def threads(self):
        return self._ex.threads

    def run(self, roots, actions, *args, allow_live=False, **kwargs):
        if SEARCH and not allow_live:
            listed = roots if isinstance(roots, (list, tuple)) else [roots]
            if not all(getattr(r, "is_world", False) for r in listed):
                warnings.warn(_LIVE_ROOTS, UserWarning, stacklevel=2)
        return self._ex.run(roots, actions, *args, **kwargs)


_GRID = GridExecutor


def __getattr__(name):
    # GridExecutor is resolved here, not bound at import, so every use passes the reminder.
    if name == "GridExecutor":
        if not os.environ.get("NUMBRION_SEARCH_PREREQS_DONE"):
            warnings.warn(_SEARCH_PREREQUISITES, UserWarning, stacklevel=2)
        return _GRID
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


del GridExecutor

__all__ = [
    "BatchEnv",
    "ENCODER",
    "Battle",
    "ChoiceError",
    "GridExecutor",
    "FORCED_MOVE",
    "N_ACTIONS",
    "N_MOVE_ACTIONS",
    "PASS",
    "SEARCH",
    "SWITCH_BASE",
    "TARGET_AUTO",
    "decode_action",
    "describe_action",
    "grid_actions",
    "move_action",
    "sample_masked",
    "sample_uniform",
    "switch_action",
]

if ENCODER:
    __all__ += ["ObsEncoder", "SmartAgent"]
