"""Chamfer both ends of every cylindrical hole wider than 2 mm."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from solidcraft import App


def main() -> None:
    app = App()
    found = app.run("inspect.find_holes", {"min_diameter": "2 mm"})
    holes = found.get("holes") or []
    for h in holes:
        app.run("solid.chamfer", {"edges": h.get("edges") or [], "distance": "1 mm"})


if __name__ == "__main__":
    main()
