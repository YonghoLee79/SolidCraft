"""Measure every STEP in a folder and write mass.csv. Usage: batch_mass.py [dir] [out.csv]"""

from __future__ import annotations

import csv
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from solidcraft import App


def is_step(path: Path) -> bool:
    return path.suffix.lower() in {".step", ".stp"}


def main() -> None:
    folder = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    out = Path(sys.argv[2] if len(sys.argv) > 2 else "mass.csv")
    files = sorted(p for p in folder.iterdir() if p.is_file() and is_step(p))
    app = App()
    rows = [("file", "volume_mm3", "mass_g", "area_mm2")]
    for path in files:
        app.run("doc.open", {"path": str(path)})
        m = app.run("inspect.measure", {})
        t = m.get("total") or {}
        rows.append((path.name, t.get("volume_mm3", 0), t.get("mass_g", 0), t.get("area_mm2", 0)))
    with out.open("w", newline="", encoding="utf-8") as f:
        csv.writer(f).writerows(rows)


if __name__ == "__main__":
    main()
