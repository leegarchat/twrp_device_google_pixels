#!/usr/bin/env python3
"""tg_push.py — push a finished AIO installer zip to a Telegram chat.

Reads the bot token + chat ids from a gitignored JSON config
(device/google/pixels/.tg_push.json)::

    {"token": "123456:ABC...", "group": {"testers": -1001234567890}}

Usage:
    tg_push.py <zip-path> <group-name> [--config PATH]

Sends via send_document with a caption (filename, size, md5), then pins
the message with notification for all (bot needs admin pin rights —
a pin failure only warns). Requires aiogram v3 (pip install aiogram).

Bot API file limit is 50 MB — bigger files are refused with a clear
error (exit 2). Any other failure exits 1. Success prints message id.
"""

import asyncio
import hashlib
import json
import os
import sys

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


async def push(token, chat_id, path, caption):
    from aiogram import Bot
    from aiogram.types import FSInputFile

    bot = Bot(token=token)
    try:
        msg = await bot.send_document(
            chat_id, FSInputFile(path), caption=caption
        )
        try:
            # disable_notification=False (default) = everyone gets notified.
            await bot.pin_chat_message(chat_id, msg.message_id)
            pinned = True
        except Exception as e:  # noqa: BLE001 — pin needs admin rights
            print(f"[tg-push] WARNING: message sent but pin failed: {e} (bot needs admin pin rights)")
            pinned = False
        return msg.message_id, pinned
    finally:
        await bot.session.close()


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
    try:
        import aiogram  # noqa: F401 — fail fast with a clear message
    except ImportError:
        print("ERROR: aiogram v3 is required (pip install aiogram)", file=sys.stderr)
        return 2
    digest = md5_of(zip_path)
    caption = f"{os.path.basename(zip_path)}\n{size / 1048576:.1f} MB | md5: {digest}"
    print(f"[tg-push] sending {os.path.basename(zip_path)} ({size / 1048576:.1f} MB) to '{group}' ...")
    try:
        message_id, pinned = asyncio.run(push(cfg["token"], cfg["groups"][group], zip_path, caption))
    except Exception as e:  # noqa: BLE001 — report any transport/API error
        print(f"ERROR: telegram refused: {e}", file=sys.stderr)
        return 1
    print(f"[tg-push] OK, message_id={message_id}" + (", pinned with notification for all" if pinned else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
