import gzip
import os
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
POOL_PATH = ROOT / "testdata" / "pool-400.txt.gz"


def n_battles(default: int) -> int:
    """Number of battles a test plays; ``NUMBRION_TEST_BATTLES`` overrides the default."""
    return int(os.environ.get("NUMBRION_TEST_BATTLES", default))


@pytest.fixture(scope="session")
def pool_path() -> Path:
    return POOL_PATH


@pytest.fixture(scope="session")
def teams() -> list:
    with gzip.open(POOL_PATH, "rt", encoding="utf-8") as f:
        return f.read().splitlines()
