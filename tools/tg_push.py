#!/usr/bin/env python3
"""tg_push.py — push a finished AIO installer zip to a Telegram chat.

Reads the bot token + chat ids from a gitignored JSON config
(device/google/pixels/.tg_push.json)::

    {"token": "123456:ABC...", "group": {"testers": -1001234567890}}

Usage:
    tg_push.py <zip-path> <group-name,...> [--config PATH]
               [--diff PREV..CUR | --diff-from TAG] [--text "postscript"] [--repo PATH]
    tg_push.py --notify "message" <group-name,...> [--config PATH]
    tg_push.py --init [--config PATH]

<group-name,...> is one group or several comma-separated groups
(e.g. "testers" or "testers,g6"); the zip is uploaded once per group
(Bot API has no multi-chat send), each message is pinned with
notification for all.

The reserved name "admin" expands to every chat in the config's
"admin" map and sends into DMs instead of groups: --push admin.
Mixing works (--push admin,testers). Pinning in a DM usually fails
(no admin rights in private chats) — that only warns, the files
are still delivered. NOTE: "admin" is reserved; a group literally
named "admin" cannot be addressed (rename it).

--text appends a postscript to the zip message after the md5 line
(after the full-changes link line when a change list was sent).

--diff collects `git log PREV..CUR` (commit subjects + bodies) from the
repo and sends it before the zip, per group: as a text message when it
fits in CHANGES_TEXT_LIMIT chars, otherwise as a `changes_<CUR>.txt`
document with the caption "changes". build.sh -D computes PREV as the
previous reachable tag and CUR as the fresh -g tag (or HEAD).
The zip caption then carries a deep link back to that changes message
("изменения версии CUR относительно версии PREV"), so the file and
its change list stay connected no matter how the chat scrolls.

--diff-from TAG is the forced variant: collect `git log TAG..HEAD`
regardless of which tags sit in between (long file goes to
`changes_from_<TAG>.txt`). TAG must exist in the repo, otherwise this
is a hard error (no zip is sent) — a typo'd base must never silently
become a zip-only push. Mutually exclusive with --diff.

--init creates the gitignored config from .tg_push.json.example
(next to this script's tree root) if it does not exist yet.

--notify sends a plain text message (no zip, no pin, no diff) — build
status pings to admin DMs (build.sh notifies admin on success and on
failure). Mutually exclusive with the zip positional and with
--diff/--diff-from/--text.

Config schema (.tg_push.json):
    {"token": "<bot token from @BotFather>",
     "group": {"<name>": <chat id>, ...},  # "groups" also accepted
     "admin": {"<name>": <user chat id>, ...}}  # "admins" also accepted
Chat id: group/channel id (e.g. -1001234567890). The bot must be a
member of the chat; pinning needs admin pin rights.
Admin id: personal user/chat id for DMs (e.g. 123456789 — the user
must have started the bot with /start, otherwise sends fail).

Sends via send_document with a caption (filename, size, md5, optional
full-changes link back to the change-list message, [+ text]), then pins
the message with notification for all (bot needs admin pin rights — a
pin failure only warns). Requires aiogram v3 (pip install aiogram).

Bot API file limit is 50 MB — bigger files are refused with a clear
error (exit 2). Any other failure exits 1. Success prints message id.
"""

import asyncio
import hashlib
import html
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
    admins = cfg.get("admins", cfg.get("admin", {}))
    if not token:
        return None, f"empty token in {path}"
    if not isinstance(groups, dict):
        return None, f"group map must be an object in {path}"
    if not isinstance(admins, dict):
        return None, f"admin map must be an object in {path}"
    return {"token": token, "groups": groups, "admins": admins}, None


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


def resolve_tag(repo, tag):
    """Check TAG resolves to a commit in repo. Returns sha or None."""
    try:
        out = subprocess.run(
            ["git", "-C", repo, "rev-parse", "--verify", "--quiet", f"{tag}^{{commit}}"],
            capture_output=True,
            text=True,
            timeout=60,
        )
    except Exception:  # noqa: BLE001 — git missing, repo broken, ...
        return None
    if out.returncode != 0:
        return None
    return out.stdout.strip() or None


def range_names(diff_range, repo):
    """Split PREV..CUR for the 'changes of CUR vs PREV' link phrase.

    A CUR of HEAD (the --diff-from form) resolves to the short sha so
    the phrase names a real version instead of the literal HEAD; any
    git failure keeps HEAD verbatim.
    """
    base, _, cur = diff_range.partition("..")
    if cur == "HEAD":
        try:
            out = subprocess.run(
                ["git", "-C", repo, "rev-parse", "--short", "HEAD"],
                capture_output=True, text=True, timeout=60,
            )
            if out.returncode == 0 and out.stdout.strip():
                cur = out.stdout.strip()
        except Exception:  # noqa: BLE001 — git missing, repo broken, ...
            pass
    return (base or "?"), (cur or "?")


def message_link(chat_id, message_id):
    """Deep link to a message just sent in this chat.

    Supergroups/channels get the public t.me/c/ form; plain user chats
    (admin DMs) get the tg:// form clients resolve in place. Either way
    a client that cannot resolve still shows the caption text.
    """
    if chat_id < 0:
        digits = str(-chat_id)
        if digits.startswith("100"):
            digits = digits[3:]
        return f"https://t.me/c/{digits}/{message_id}"
    return f"tg://openmessage?user_id={chat_id}&message_id={message_id}"


def zip_caption(zip_name, size_mb, digest, push_text, link=None, base=None, cur=None):
    """File-message caption: name, size|md5, optional full-changes link,
    optional postscript. Always HTML (escaped), so the link phrase
    renders as a link and arbitrary --text stays literal."""
    head = f"{html.escape(zip_name)}\n{size_mb} MB | md5: {digest}"
    if link and base and cur:
        head += (
            f'\n<a href="{html.escape(link, quote=True)}">'
            f"изменения версии {html.escape(cur)} "
            f"относительно версии {html.escape(base)}</a>"
        )
    if push_text:
        head += f"\n\n{html.escape(push_text)}"
    return head


def changes_filename(range_):
    if ".." in range_:
        base, _, cur = range_.partition("..")
        # --diff-from resolves to TAG..HEAD: HEAD as CUR would name every
        # file changes_HEAD.txt, so pin the base tag instead.
        if cur == "HEAD" and base:
            safe = "".join(c if (c.isalnum() or c in "-_.") else "_" for c in base).strip("._")
            return f"changes_from_{safe or 'base'}.txt"
        cur = range_.split("..")[-1]
    else:
        cur = range_
    safe = "".join(c if (c.isalnum() or c in "-_.") else "_" for c in cur).strip("._") or "changes"
    return f"changes_{safe}.txt"


async def run_notify(token, chat_ids, text):
    """Send a plain text status message per chat (no pin). Returns failures."""
    from aiogram import Bot

    bot = Bot(token=token)
    failed = 0
    try:
        for group, chat_id in chat_ids:
            try:
                msg = await bot.send_message(chat_id, text)
                print(f"[tg-push] OK '{group}': notify message_id={msg.message_id}")
            except Exception as e:  # noqa: BLE001 — report any transport/API error
                print(f"ERROR: telegram refused notify for '{group}': {e}", file=sys.stderr)
                failed += 1
    finally:
        await bot.session.close()
    return failed


async def run_push(token, chat_ids, zip_path, zip_name, size_mb, digest,
                 push_text, diff_range, diff_repo):
    """Send the --diff change list, then zip (+pin) per chat. Returns failures.

    The zip caption links back to the change-list message sent just
    above in the same chat (message_id is only known after that send,
    so the caption is built per chat, after the changes land).
    """
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
    base, cur = range_names(diff_range, diff_repo) if diff_range else (None, None)
    try:
        for group, chat_id in chat_ids:
            changes_mid = None
            if changes_file:
                try:
                    doc = await bot.send_document(
                        chat_id,
                        FSInputFile(changes_file, filename=changes_filename(diff_range)),
                        caption=f"changes {diff_range}",
                    )
                    print(f"[tg-push] OK '{group}': changes file message_id={doc.message_id}")
                    changes_mid = doc.message_id
                except Exception as e:  # noqa: BLE001
                    print(f"ERROR: telegram refused changes file for '{group}': {e}", file=sys.stderr)
                    failed += 1
            elif changes_text:
                try:
                    txt = await bot.send_message(chat_id, f"Changes {diff_range}:\n\n{changes_text}")
                    print(f"[tg-push] OK '{group}': changes message_id={txt.message_id}")
                    changes_mid = txt.message_id
                except Exception as e:  # noqa: BLE001
                    print(f"ERROR: telegram refused changes text for '{group}': {e}", file=sys.stderr)
                    failed += 1

            link = message_link(chat_id, changes_mid) if changes_mid else None
            caption = zip_caption(zip_name, size_mb, digest, push_text, link, base, cur)
            print(f"[tg-push] sending {os.path.basename(zip_path)} to '{group}' ...")
            try:
                msg = await bot.send_document(
                    chat_id, FSInputFile(zip_path), caption=caption, parse_mode="HTML")
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
    diff_from = None
    push_text = None
    notify_text = None
    repo_path = None
    positional = []
    do_init = False
    skip_next = False
    value_flags = {"--config", "--diff", "--diff-from", "--text", "--repo", "--notify"}
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
            elif a == "--diff-from":
                diff_from = argv[i + 1]
            elif a == "--text":
                push_text = argv[i + 1]
            elif a == "--repo":
                repo_path = argv[i + 1]
            elif a == "--notify":
                notify_text = argv[i + 1]
            skip_next = True
        elif a == "--init":
            do_init = True
        elif a.startswith("--"):
            print(f"unknown option: {a}", file=sys.stderr)
            return 2
        else:
            positional.append(a)
    if do_init:
        if positional or diff_range or diff_from or push_text or notify_text:
            print("usage: tg_push.py --init [--config PATH]", file=sys.stderr)
            return 2
        return cmd_init(cfg_path or default_config_path())
    if notify_text is not None:
        if diff_range or diff_from or push_text:
            print("ERROR: --notify is mutually exclusive with --diff/--diff-from/--text", file=sys.stderr)
            return 2
        if len(positional) != 1:
            print("usage: tg_push.py --notify \"message\" <group-name,...> [--config PATH]", file=sys.stderr)
            return 2
        groups_arg = positional[0]
        notify_groups = [g.strip() for g in groups_arg.split(",") if g.strip()]
        if not notify_groups:
            print("ERROR: no group names given", file=sys.stderr)
            return 2
        if cfg_path is None:
            cfg_path = default_config_path()
        cfg, err = load_config(cfg_path)
        if err is not None:
            print(f"ERROR: {err}", file=sys.stderr)
            return 2
        expanded = []
        for g in notify_groups:
            if g == "admin":
                if not cfg["admins"]:
                    print(f"ERROR: 'admin' requested but no admins in {cfg_path} "
                          f"(add an \"admin\" map of user chat ids)", file=sys.stderr)
                    return 2
                expanded.extend((f"admin:{name}", cid) for name, cid in sorted(cfg["admins"].items()))
            else:
                expanded.append((g, None))
        unknown = [g for g, _ in expanded if not g.startswith("admin:") and g not in cfg["groups"]]
        if unknown:
            print(
                f"ERROR: unknown group(s) {', '.join(unknown)} in {cfg_path} "
                f"(have: {', '.join(sorted(cfg['groups'])) or '<none>'})",
                file=sys.stderr,
            )
            return 2
        chat_ids = [
            (label, cid if cid is not None else cfg["groups"][label])
            for label, cid in expanded
        ]
        try:
            import aiogram  # noqa: F401 — fail fast with a clear message
        except ImportError:
            print("ERROR: aiogram v3 is required (pip install aiogram)", file=sys.stderr)
            return 2
        try:
            failed = asyncio.run(run_notify(cfg["token"], chat_ids, notify_text))
        except Exception as e:  # noqa: BLE001 — aiogram init / event loop failure
            print(f"ERROR: telegram client failed: {e}", file=sys.stderr)
            return 1
        if failed:
            print(f"[tg-push] failures: {failed}", file=sys.stderr)
            return 1
        return 0
    if len(positional) != 2:
        print("usage: tg_push.py <zip-path> <group-name,...> [--config PATH] [--diff PREV..CUR | --diff-from TAG] [--text \"...\"] [--repo PATH]", file=sys.stderr)
        print("       tg_push.py --notify \"message\" <group-name,...> [--config PATH]", file=sys.stderr)
        print("       tg_push.py --init [--config PATH]", file=sys.stderr)
        return 2
    zip_path, groups_arg = positional
    groups = [g.strip() for g in groups_arg.split(",") if g.strip()]
    if not groups:
        print("ERROR: no group names given", file=sys.stderr)
        return 2
    if cfg_path is None:
        cfg_path = default_config_path()
    if diff_range and diff_from:
        print("ERROR: --diff and --diff-from are mutually exclusive", file=sys.stderr)
        return 2
    if diff_range and ".." not in diff_range:
        print("ERROR: --diff needs a PREV..CUR range", file=sys.stderr)
        return 2
    if diff_from:
        if not diff_from.strip() or ".." in diff_from:
            print("ERROR: --diff-from needs a single tag name", file=sys.stderr)
            return 2
        if resolve_tag(repo_path or default_repo_path(), diff_from) is None:
            print(f"ERROR: --diff-from tag not found: {diff_from}", file=sys.stderr)
            return 2
        diff_range = f"{diff_from}..HEAD"
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
    # Reserved name "admin" = every DM in the admin map (see docstring).
    expanded = []
    for g in groups:
        if g == "admin":
            if not cfg["admins"]:
                print(f"ERROR: 'admin' requested but no admins in {cfg_path} "
                      f"(add an \"admin\" map of user chat ids)", file=sys.stderr)
                return 2
            expanded.extend((f"admin:{name}", cid) for name, cid in sorted(cfg["admins"].items()))
        else:
            expanded.append((g, None))
    unknown = [g for g, _ in expanded if not g.startswith("admin:") and g not in cfg["groups"]]
    if unknown:
        print(
            f"ERROR: unknown group(s) {', '.join(unknown)} in {cfg_path} "
            f"(have: {', '.join(sorted(cfg['groups'])) or '<none>'})",
            file=sys.stderr,
        )
        return 2
    chat_ids = [
        (label, cid if cid is not None else cfg["groups"][label])
        for label, cid in expanded
    ]
    try:
        import aiogram  # noqa: F401 — fail fast with a clear message
    except ImportError:
        print("ERROR: aiogram v3 is required (pip install aiogram)", file=sys.stderr)
        return 2
    digest = md5_of(zip_path)
    try:
        failed = asyncio.run(run_push(
            cfg["token"], chat_ids, os.path.abspath(zip_path),
            os.path.basename(zip_path), f"{size / 1048576:.1f}", digest,
            push_text, diff_range, repo_path or default_repo_path(),
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
