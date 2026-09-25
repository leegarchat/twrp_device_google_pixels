#!/usr/bin/env python3
"""tg_push.py — push a finished AIO installer zip to a Telegram chat.

Reads the bot token + chat ids from a gitignored JSON config
(device/google/pixels/.tg_push.json)::

    {"token": "123456:ABC...", "group": {"testers": -1001234567890}}

Usage:
    tg_push.py <zip-path> <group-name,...> [--config PATH]
               [--diff PREV..CUR] [--text "postscript"] [--repo PATH]
    tg_push.py --init [--config PATH]

<group-name,...> is one group or several comma-separated groups
(e.g. "testers" or "testers,g6"); the zip is uploaded once per group
(Bot API has no multi-chat send), each message is pinned with
notification for all.

--text appends a postscript to the zip message after the md5 line.

--diff collects `git log PREV..CUR` (commit subjects + bodies) from the
repo and sends it before the zip, per group: as a text message when it
fits in CHANGES_TEXT_LIMIT chars, otherwise as a `changes_<CUR>.txt`
document with the caption "changes". build.sh -D computes PREV as the
previous reachable tag and CUR as the fresh -g tag (or HEAD).

--init creates the gitignored config from .tg_push.json.example
(next to this script's tree root) if it does not exist yet.

Config schema (.tg_push.json):
    {"token": "<bot token from @BotFather>",
     "group": {"<name>": <chat id>, ...}}   # "groups" also accepted
Chat id: group/channel id (e.g. -1001234567890). The bot must be a
member of the chat; pinning needs admin pin rights.

Sends via send_document with a caption (filename, size, md5 [+ text]),
then pins the message with notification for all (bot needs admin pin
rights — a pin failure only warns). Requires aiogram v3
(pip install aiogram).

Bot API file limit is 50 MB — bigger files are refused with a clear
error (exit 2). Any other failure exits 1. Success prints message id.
"""

import asyncio
import hashlib
import json
import os
import subprocess
import sys
import tempfile

TG_API_LIMIT = 50 * 1024 * 1024  # Bot API per-file cap
CHANGES_TEXT_LIMIT = 2400  # longer change lists go to a .txt document


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


def default_config_path():
    return os.path.normpath(
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".tg_push.json")
    )


def default_repo_path():
    # tools/ lives in the device tree root: tools/.. == the git repo.
    return os.path.normpath(
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    )


def cmd_init(cfg_path):
    import shutil

    if os.path.isfile(cfg_path):
        print(f"config already exists: {cfg_path}")
        return 0
    example = os.path.join(os.path.dirname(default_config_path()), ".tg_push.json.example")
    if not os.path.isfile(example):
        print(f"ERROR: example not found: {example}", file=sys.stderr)
        return 2
    shutil.copy(example, cfg_path)
    os.chmod(cfg_path, 0o600)
    print(f"created {cfg_path} (mode 600) — fill in token + chat ids")
    return 0


def md5_of(path):
    h = hashlib.md5()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(4 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def collect_diff(repo, range_):
    """Run `git log PREV..CUR` in repo. Returns (text, error)."""
    try:
        out = subprocess.run(
            ["git", "-C", repo, "log", range_, "--pretty=format:- %h %ad %s%n%b", "--date=short"],
            capture_output=True,
            text=True,
            timeout=120,
        )
    except Exception as e:  # noqa: BLE001 — git missing, repo broken, ...
        return None, str(e)
    if out.returncode != 0:
        return None, (out.stderr.strip() or f"git log {range_} failed").splitlines()[0][:200]
    return out.stdout.strip(), None


def changes_filename(range_):
    cur = range_.split("..")[-1] if ".." in range_ else range_
    safe = "".join(c if (c.isalnum() or c in "-_.") else "_" for c in cur).strip("._") or "changes"
    return f"changes_{safe}.txt"


async def run_push(token, chat_ids, zip_path, caption, diff_range, diff_repo):
    """Send the --diff change list, then zip (+pin) per chat. Returns failures."""
    from aiogram import Bot
    from aiogram.types import FSInputFile

    # Collect + stage the change list BEFORE any network, so a bad range
    # fails fast and the .txt decision is made up front.
    changes_text, changes_file = None, None
    if diff_range:
        changes_text, err = collect_diff(diff_repo, diff_range)
        if err is not None:
            print(f"[tg-push] WARNING: diff {diff_range} failed ({err}), sending zip only)")
        elif not changes_text:
            print(f"[tg-push] diff {diff_range} is empty, sending zip only")
            changes_text = None
        elif len(changes_text) > CHANGES_TEXT_LIMIT:
            fd, changes_file = tempfile.mkstemp(prefix="tg_changes_", suffix=".txt")
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(changes_text + "\n")
            print(f"[tg-push] changes are {len(changes_text)} chars, staged {changes_filename(diff_range)}")
            changes_text = None
        else:
            print(f"[tg-push] changes are {len(changes_text)} chars, will send as text")

    bot = Bot(token=token)
    failed = 0
    try:
        for group, chat_id in chat_ids:
            if changes_file:
                try:
                    doc = await bot.send_document(
                        chat_id,
                        FSInputFile(changes_file, filename=changes_filename(diff_range)),
                        caption=f"changes {diff_range}",
                    )
                    print(f"[tg-push] OK '{group}': changes file message_id={doc.message_id}")
                except Exception as e:  # noqa: BLE001
                    print(f"ERROR: telegram refused changes file for '{group}': {e}", file=sys.stderr)
                    failed += 1
            elif changes_text:
                try:
                    txt = await bot.send_message(chat_id, f"Changes {diff_range}:\n\n{changes_text}")
                    print(f"[tg-push] OK '{group}': changes message_id={txt.message_id}")
                except Exception as e:  # noqa: BLE001
                    print(f"ERROR: telegram refused changes text for '{group}': {e}", file=sys.stderr)
                    failed += 1

            print(f"[tg-push] sending {os.path.basename(zip_path)} to '{group}' ...")
            try:
                msg = await bot.send_document(chat_id, FSInputFile(zip_path), caption=caption)
                try:
                    # disable_notification=False (default) = everyone gets notified.
                    await bot.pin_chat_message(chat_id, msg.message_id)
                    pinned = True
                except Exception as e:  # noqa: BLE001 — pin needs admin rights
                    print(f"[tg-push] WARNING: message sent but pin failed: {e} (bot needs admin pin rights)")
                    pinned = False
                print(f"[tg-push] OK '{group}': message_id={msg.message_id}" + (", pinned with notification for all" if pinned else ""))
            except Exception as e:  # noqa: BLE001 — report any transport/API error
                print(f"ERROR: telegram refused zip for '{group}': {e}", file=sys.stderr)
                failed += 1
                continue
    finally:
        await bot.session.close()
        if changes_file:
            try:
                os.unlink(changes_file)
            except OSError:
                pass
    return failed


def main(argv):
    cfg_path = None
    diff_range = None
    push_text = None
    repo_path = None
    positional = []
    do_init = False
    skip_next = False
    value_flags = {"--config", "--diff", "--text", "--repo"}
    for i, a in enumerate(argv):
        if skip_next:
            skip_next = False
            continue
        if a in value_flags:
            if i + 1 >= len(argv):
                print(f"ERROR: {a} requires a value", file=sys.stderr)
                return 2
            if a == "--config":
                cfg_path = argv[i + 1]
            elif a == "--diff":
                diff_range = argv[i + 1]
            elif a == "--text":
                push_text = argv[i + 1]
            elif a == "--repo":
                repo_path = argv[i + 1]
            skip_next = True
        elif a == "--init":
            do_init = True
        elif a.startswith("--"):
            print(f"unknown option: {a}", file=sys.stderr)
            return 2
        else:
            positional.append(a)
    if do_init:
        if positional or diff_range or push_text:
            print("usage: tg_push.py --init [--config PATH]", file=sys.stderr)
            return 2
        return cmd_init(cfg_path or default_config_path())
    if len(positional) != 2:
        print("usage: tg_push.py <zip-path> <group-name,...> [--config PATH] [--diff PREV..CUR] [--text \"...\"] [--repo PATH]", file=sys.stderr)
        print("       tg_push.py --init [--config PATH]", file=sys.stderr)
        return 2
    zip_path, groups_arg = positional
    groups = [g.strip() for g in groups_arg.split(",") if g.strip()]
    if not groups:
        print("ERROR: no group names given", file=sys.stderr)
        return 2
    if cfg_path is None:
        cfg_path = default_config_path()
    if diff_range and ".." not in diff_range:
        print("ERROR: --diff needs a PREV..CUR range", file=sys.stderr)
        return 2
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
    unknown = [g for g in groups if g not in cfg["groups"]]
    if unknown:
        print(
            f"ERROR: unknown group(s) {', '.join(unknown)} in {cfg_path} "
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
    if push_text:
        caption += f"\n\n{push_text}"
    chat_ids = [(g, cfg["groups"][g]) for g in groups]
    try:
        failed = asyncio.run(run_push(
            cfg["token"], chat_ids, os.path.abspath(zip_path), caption,
            diff_range, repo_path or default_repo_path(),
        ))
    except Exception as e:  # noqa: BLE001 — aiogram init / event loop failure
        print(f"ERROR: telegram client failed: {e}", file=sys.stderr)
        return 1
    if failed:
        print(f"[tg-push] failures: {failed}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
