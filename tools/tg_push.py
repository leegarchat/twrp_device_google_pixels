#!/usr/bin/env python3
"""tg_push.py — push a finished AIO installer zip to a Telegram chat.

Reads the bot token + chat ids from a gitignored JSON config
(device/google/pixels/.tg_push.json)::

    {"token": "123456:ABC...", "group": {"testers": -1001234567890}}

Usage:
    tg_push.py <zip-path> <group-name> [--config PATH]

Only stdlib is used (urllib). Sends via sendDocument with a caption
(filename, size, md5). Bot API file limit is 50 MB — bigger files are
refused with a clear error (exit 2). Any other failure exits 1 with the
API error text. Success prints the message id.
"""

import hashlib
import json
import os
import sys
import urllib.request

TG_API_LIMIT = 50 * 1024 * 1024  # Bot API per-file cap


def load_config(path):
    try:
        with open(path, "r", encoding="utf-8") as f:
            cfg = json.load(f)
    except FileNotFoundError:
        return None, f"config not found: {path} (copy .tg_push.json.example and fill in)"
    except json.JSONDecodeError as e:
        return None, f"bad JSON in {path}: {e}"
    token = cfg.get("token", "")
    groups = cfg.get("groups", cfg.get("group", {}))
    if not token:
        return None, f"empty token in {path}"
    if not isinstance(groups, dict):
        return None, f"group map must be an object in {path}"
    return {"token": token, "groups": groups}, None


def md5_of(path):
    h = hashlib.md5()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(4 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def send_document(token, chat_id, path, caption):
    boundary = "----foxpush7d4a6e8c0b2"
    crlf = b"\r\n"
    filename = os.path.basename(path)
    with open(path, "rb") as f:
        data = f.read()
    parts = []
    for name, value in (("chat_id", str(chat_id)), ("caption", caption)):
        parts += [
            b"--" + boundary.encode() + crlf,
            f'Content-Disposition: form-data; name="{name}"'.encode() + crlf + crlf,
            value.encode() + crlf,
        ]
    parts += [
        b"--" + boundary.encode() + crlf,
        f'Content-Disposition: form-data; name="document"; filename="{filename}"'.encode() + crlf,
        b"Content-Type: application/zip" + crlf + crlf,
        data + crlf,
        b"--" + boundary.encode() + b"--" + crlf,
    ]
    body = b"".join(parts)
    req = urllib.request.Request(
        f"https://api.telegram.org/bot{token}/sendDocument",
        data=body,
        headers={
            "Content-Type": f"multipart/form-data; boundary={boundary}",
            "Content-Length": str(len(body)),
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=300) as resp:
            return json.load(resp)
    except Exception as e:  # noqa: BLE001 — report any transport/API error
        return {"ok": False, "description": str(e)}


def main(argv):
    cfg_path = None
    positional = []
    skip_next = False
    for i, a in enumerate(argv):
        if skip_next:
            skip_next = False
            continue
        if a == "--config" and i + 1 < len(argv):
            cfg_path = argv[i + 1]
            skip_next = True
        elif a.startswith("--"):
            print(f"unknown option: {a}", file=sys.stderr)
            return 2
        else:
            positional.append(a)
    if len(positional) != 2:
        print("usage: tg_push.py <zip-path> <group-name> [--config PATH]", file=sys.stderr)
        return 2
    zip_path, group = positional
    if cfg_path is None:
        cfg_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".tg_push.json")
        cfg_path = os.path.normpath(cfg_path)
    if not os.path.isfile(zip_path):
        print(f"ERROR: zip not found: {zip_path}", file=sys.stderr)
        return 2
    size = os.path.getsize(zip_path)
    if size > TG_API_LIMIT:
        print(
            f"ERROR: {os.path.basename(zip_path)} is {size / 1048576:.1f} MB, "
            f"Bot API cap is {TG_API_LIMIT / 1048576:.0f} MB — upload manually.",
            file=sys.stderr,
        )
        return 2
    cfg, err = load_config(cfg_path)
    if err is not None:
        print(f"ERROR: {err}", file=sys.stderr)
        return 2
    if group not in cfg["groups"]:
        print(
            f"ERROR: unknown group '{group}' in {cfg_path} "
            f"(have: {', '.join(sorted(cfg['groups'])) or '<none>'})",
            file=sys.stderr,
        )
        return 2
    digest = md5_of(zip_path)
    caption = (
        f"{os.path.basename(zip_path)}\n"
        f"{size / 1048576:.1f} MB | md5: {digest}"
    )
    print(f"[tg-push] sending {os.path.basename(zip_path)} ({size / 1048576:.1f} MB) to '{group}' ...")
    resp = send_document(cfg["token"], cfg["groups"][group], zip_path, caption)
    if not resp.get("ok"):
        print(f"ERROR: telegram refused: {resp.get('description', resp)}", file=sys.stderr)
        return 1
    print(f"[tg-push] OK, message_id={resp['result']['message_id']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
