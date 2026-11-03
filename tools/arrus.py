"""
arrus.py — read/write ArrangeUs `.arrus` choreography files.

File format (ArrangeUs Android v2.9.4, package com.AndriyStefanchuk.ArrangeUs.main):

    file = IV(16 bytes) || ciphertext

    ciphertext = AES-256-CBC( PKCS7(utf8(json)) )
      key = the app's fixed 32-byte secret (not in this repo: set ARUS_FORMAT_KEY
            or put it in a .arrus-key file)
      iv  = the 16 random bytes prepended to the file

    plaintext = a UTF-8 JSON document (one Choreography; see schema in README)

The same key/scheme is used for single choreographies (.arrus) and for full
backups (the app's `parseBackup`), which wrap a different JSON shape.

Recovered by decompiling the app's Dart AOT snapshot (utilities/data_encryptor.dart).
The key is a fixed, app-global secret, so every file the app writes uses it.
"""

from __future__ import annotations

import json
import os
from typing import Any

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

def _load_key() -> bytes:
    k = os.environ.get("ARUS_FORMAT_KEY")
    if k:
        return k.rstrip("\r\n").encode()
    d = os.path.abspath(os.getcwd())
    while True:
        f = os.path.join(d, ".arrus-key")
        if os.path.exists(f):
            with open(f, "rb") as fh:
                return fh.read().rstrip(b"\r\n")
        parent = os.path.dirname(d)
        if parent == d:
            raise SystemExit("no key: set ARUS_FORMAT_KEY or create .arrus-key")
        d = parent


KEY = _load_key()
IV_LEN = 16
BLOCK = 16


def _pkcs7_unpad(data: bytes) -> bytes:
    if not data or len(data) % BLOCK != 0:
        raise ValueError("ciphertext not block-aligned")
    n = data[-1]
    if n < 1 or n > BLOCK or data[-n:] != bytes([n]) * n:
        raise ValueError("invalid PKCS7 padding (wrong key or corrupt file)")
    return data[:-n]


def _pkcs7_pad(data: bytes) -> bytes:
    n = BLOCK - (len(data) % BLOCK)
    return data + bytes([n]) * n


def decrypt(blob: bytes) -> str:
    """Decrypt raw .arrus bytes -> JSON text."""
    if len(blob) < IV_LEN + BLOCK:
        raise ValueError("file too small to be a valid .arrus")
    iv, ct = blob[:IV_LEN], blob[IV_LEN:]
    pt = Cipher(algorithms.AES(KEY), modes.CBC(iv)).decryptor().update(ct)
    return _pkcs7_unpad(pt).decode("utf-8")


def encrypt(text: str, iv: bytes | None = None) -> bytes:
    """Encrypt JSON text -> raw .arrus bytes (round-trips with the app)."""
    iv = iv if iv is not None else os.urandom(IV_LEN)
    if len(iv) != IV_LEN:
        raise ValueError("iv must be 16 bytes")
    ct = Cipher(algorithms.AES(KEY), modes.CBC(iv)).encryptor().update(
        _pkcs7_pad(text.encode("utf-8"))
    )
    return iv + ct


def load(path: str) -> dict[str, Any]:
    """Read a .arrus file -> parsed JSON object."""
    with open(path, "rb") as f:
        return json.loads(decrypt(f.read()))


def save(obj: dict[str, Any], path: str) -> None:
    """Write a JSON object -> .arrus file."""
    with open(path, "wb") as f:
        f.write(encrypt(json.dumps(obj, separators=(",", ":"))))


if __name__ == "__main__":
    import sys

    if len(sys.argv) < 2:
        sys.exit("usage: python arrus.py <file.arrus> [out.json]")
    data = load(sys.argv[1])
    out = json.dumps(data, indent=2, ensure_ascii=False)
    if len(sys.argv) > 2:
        with open(sys.argv[2], "w", encoding="utf-8") as f:
            f.write(out)
        print(f"wrote {sys.argv[2]}")
    else:
        print(out)
