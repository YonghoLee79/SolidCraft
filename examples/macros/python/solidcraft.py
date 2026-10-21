"""Drive a live SolidCraft window over the control port (stdlib only)."""

from __future__ import annotations

import json
import os
import socket
import sys
from pathlib import Path


def _token_file(port: int) -> Path | None:
    if d := os.environ.get("SOLIDCRAFT_CONTROL_DIR"):
        return Path(d) / f"{port}.token"
    if os.name == "nt":
        root = os.environ.get("LOCALAPPDATA")
        return Path(root) / "SolidCraft" / "control" / f"{port}.token" if root else None
    home = Path.home()
    if sys.platform == "darwin":
        return home / "Library" / "Application Support" / "SolidCraft" / "control" / f"{port}.token"
    xdg = os.environ.get("XDG_DATA_HOME")
    base = Path(xdg) if xdg else home / ".local" / "share"
    return base / "solidcraft" / "control" / f"{port}.token"


def _token(port: int) -> str | None:
    env = os.environ.get("SOLIDCRAFT_CONTROL_TOKEN", "").strip()
    if len(env) >= 32:
        return env
    path = _token_file(port)
    if path is None or not path.is_file():
        return None
    text = path.read_text(encoding="utf-8").strip()
    return text or None


class App:
    def __init__(self) -> None:
        port = int(os.environ.get("SOLIDCRAFT_CONTROL_PORT") or "0")
        if port == 0:
            raise SystemExit("SOLIDCRAFT_CONTROL_PORT is not set (Play python-macro from MACRO)")
        self._sock = socket.create_connection(("127.0.0.1", port))
        self._r = self._sock.makefile("r", encoding="utf-8", newline="\n")
        self._n = 1
        self._token = _token(port)

    def run(self, command: str, params: dict | None = None) -> dict:
        msg: dict = {"id": self._n, "method": "engine.execute", "params": {"command": command, "params": params or {}}}
        if self._n == 1 and self._token:
            msg["token"] = self._token
        self._n += 1
        self._sock.sendall((json.dumps(msg) + "\n").encode())
        line = self._r.readline()
        if not line:
            raise SystemExit("SolidCraft closed the connection")
        reply = json.loads(line)
        if not reply.get("ok"):
            raise SystemExit(reply.get("error") or "error")
        result = reply.get("result")
        return result if isinstance(result, dict) else {}
