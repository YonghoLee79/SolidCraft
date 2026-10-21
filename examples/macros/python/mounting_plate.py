"""Rectangular plate, 8 mm thick, M8 clearance holes 12 mm from the edges.

Target pitch 50 mm, even spacing. A gap shorter than 40 mm drops a hole.
Asks for length and width unless they are given on the command line.
Usage: mounting_plate.py [length_mm] [width_mm]
       mounting_plate.py --count [length_mm] [width_mm]  (print hole count, no CAD)
"""

from __future__ import annotations

import sys
from pathlib import Path

OFFSET = 12.0
PITCH = 50.0
MIN_PITCH = 40.0
THICK = 8.0


def gaps(span: float) -> int:
    if span < 0:
        raise SystemExit("plate too small for a 12 mm edge offset")
    n = max(1, round(span / PITCH))
    while n > 1 and span / n < MIN_PITCH:
        n -= 1
    return n


def along(length: float) -> list[float]:
    span = length - 2 * OFFSET
    n = gaps(span)
    step = span / n
    return [OFFSET + i * step for i in range(n + 1)]


def perimeter(length: float, width: float) -> list[tuple[float, float]]:
    xs = along(length)
    ys = along(width)
    pts = [(x, OFFSET) for x in xs]
    pts += [(x, width - OFFSET) for x in xs]
    pts += [(OFFSET, y) for y in ys[1:-1]]
    pts += [(length - OFFSET, y) for y in ys[1:-1]]
    return pts


def ask_size() -> tuple[float, float]:
    try:
        import tkinter as tk
    except ImportError:
        raise SystemExit("give length_mm and width_mm, or install tkinter") from None

    root = tk.Tk()
    root.title("Mounting plate")
    root.resizable(False, False)
    result: dict[str, float | None] = {"length": None, "width": None}

    tk.Label(root, text="Length (mm)").grid(row=0, column=0, padx=8, pady=6, sticky="e")
    length_var = tk.StringVar(value="200")
    tk.Entry(root, textvariable=length_var, width=12).grid(row=0, column=1, padx=8, pady=6)
    tk.Label(root, text="Width (mm)").grid(row=1, column=0, padx=8, pady=6, sticky="e")
    width_var = tk.StringVar(value="150")
    tk.Entry(root, textvariable=width_var, width=12).grid(row=1, column=1, padx=8, pady=6)

    def ok() -> None:
        try:
            result["length"] = float(length_var.get())
            result["width"] = float(width_var.get())
        except ValueError:
            return
        if result["length"] > 24 and result["width"] > 24:
            root.destroy()

    tk.Button(root, text="OK", command=ok).grid(row=2, column=0, padx=8, pady=8)
    tk.Button(root, text="Cancel", command=root.destroy).grid(row=2, column=1, padx=8, pady=8)
    root.mainloop()
    if result["length"] is None or result["width"] is None:
        raise SystemExit("cancelled")
    return result["length"], result["width"]


def main() -> None:
    args = [a for a in sys.argv[1:] if a != "--count"]
    count_only = "--count" in sys.argv[1:]
    if len(args) >= 2:
        length, width = float(args[0]), float(args[1])
    elif count_only:
        length, width = 200.0, 150.0
    else:
        length, width = ask_size()
    pts = perimeter(length, width)
    if count_only:
        print(len(pts))
        return
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from solidcraft import App

    app = App()
    app.run("solid.box", {"length": f"{length} mm", "width": f"{width} mm", "height": f"{THICK} mm", "name": "Plate"})
    for i, (x, y) in enumerate(pts, start=1):
        app.run(
            "solid.hole",
            {"position": [x, y, THICK], "clearance": "M8", "name": f"M8_{i}"},
        )


if __name__ == "__main__":
    main()
