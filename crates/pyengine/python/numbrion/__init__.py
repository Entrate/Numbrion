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
from ._numbrion import GridExecutor as _GridExecutor
from .utils import grid_actions, sample_masked, sample_uniform  # noqa: F401

_SEARCH_PREREQUISITES = (
    "numbrion.GridExecutor: search prerequisites are not done yet - fair-information determinization "
    "(search must not see the opponent's true hidden sets) and the MSVC + PGO build. Experiments only, "
    "not training targets or strength claims. See docs/design/SEARCH-PREREQUISITES.md; set "
    "NUMBRION_SEARCH_PREREQS_DONE=1 once both are done."
)


def __getattr__(name):
    # GridExecutor is resolved here, not imported above, so every use passes the reminder.
    if name == "GridExecutor":
        if not os.environ.get("NUMBRION_SEARCH_PREREQS_DONE"):
            warnings.warn(_SEARCH_PREREQUISITES, UserWarning, stacklevel=2)
        return _GridExecutor
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

__all__ = [
    "BatchEnv",
    "Battle",
    "ChoiceError",
    "GridExecutor",
    "FORCED_MOVE",
    "N_ACTIONS",
    "N_MOVE_ACTIONS",
    "PASS",
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
