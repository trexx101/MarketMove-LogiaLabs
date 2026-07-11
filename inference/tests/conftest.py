"""Pytest configuration for the inference package.

Adds the repo root to ``sys.path`` so tests can import the ``inference``
package regardless of where pytest is invoked from. This mirrors the
``PYTHONPATH=/home/ubuntu/projects/MarketMoves`` invocation used in the
feature spec and CI.
"""
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))
