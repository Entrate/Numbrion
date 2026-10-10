"""Training interface for the numbrion ``gen9randomdoublesbattle`` engine.

``Battle`` is one battle (debugging, scripted play, search); ``BatchEnv`` runs many in parallel with
auto-reset. See ``docs/design/TRAINING-API.md`` for the action encoding, masks and observation rules.
"""

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
from .utils import sample_masked, sample_uniform  # noqa: F401

__all__ = [
    "BatchEnv",
    "Battle",
    "ChoiceError",
    "FORCED_MOVE",
    "N_ACTIONS",
    "N_MOVE_ACTIONS",
    "PASS",
    "SWITCH_BASE",
    "TARGET_AUTO",
    "decode_action",
    "describe_action",
    "move_action",
    "sample_masked",
    "sample_uniform",
    "switch_action",
]
