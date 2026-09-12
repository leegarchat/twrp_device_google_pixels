#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
sync_tree.py — smart sync of the Android source tree while preserving
local changes (micro-patches), plus a snapshot manager.

Modes:
  (no flags)     sync using the file list from bakFiles/original
  -s, --smart    deep scanner of all git projects in the tree (via the repo manifest)
  -d, --diff     (only with -s) only take a snapshot of the changes, no repo sync
  -c, --change   interactive snapshot and patch manager
  -f, --forcesync  full force sync with forced revert of changes,
                   without re-applying patches afterwards (a snapshot is ALWAYS taken:
                   without -s — from bakFiles/original, with -s — deep scan);
                   if nothing is found in original/ — offers a deep -s scan
  -r, --revert   ONLY revert local changes to HEAD, no repo sync
                   (a snapshot is ALWAYS taken as a safety net, patches are NOT
                   re-applied; without -s — from bakFiles/original, with -s — deep scan)
  -e, --except   smart-scanner exclusion management UI
  -j, --jobs N   repo sync thread count
  -y, --yes      non-interactive mode (auto-confirm all prompts)
  --ofox-sync    use the official OrangeFox update flow instead of plain repo sync:
                 sync all manifest projects except manual OFOX clones
                 (bootable/recovery), then `git pull` bootable/recovery,
                 vendor/recovery and external/se_omapi (official update_fox.sh
                 scheme; android_bootable_recovery checkout errors are expected
                 and skipped by design)
  --ofox-setup   OrangeFox sync configuration UI (sync tools dir, branch
                 14.1/12.1, default flow); stored in bakFiles/ofox_sync.json
  --ofox-tools PATH / --ofox-branch {14.1,12.1}
                 one-shot config overrides (also saved)

bakFiles/ layout:
  original/                 clean HEAD copies of tracked files (*.bp/*.mk -> +.bak)
  smart_exceptions.json     user-defined smart-scanner exclusions
  snapshots/<ts>/
    modified/               backup of CURRENT modified files (*.bp/*.mk -> +.bak)
    original/               clean HEAD copies at snapshot time (*.bp/*.mk -> +.bak)
    patches/<rel>.patch     unified diff (a/<rel> -> b/<rel>)
    new_files/              untracked files temporarily removed from the tree
    original_prev/          previous bakFiles/original content (after smart cleanup)
    manifest.json           snapshot description and results
"""

from __future__ import annotations

import os
import sys
import json
import shutil
import signal
import argparse
import subprocess
import tempfile
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from datetime import datetime
from typing import Optional, Tuple, List, Dict, Set

# ============================================================
# 1. Emergency exit (SIGINT / SIGTERM)
# ============================================================
ACTIVE_TEMP_PATHS: Set[Path] = set()
_res_lock = threading.Lock()


def emergency_cleanup(signum, frame):
    with _res_lock:
        for tmp in list(ACTIVE_TEMP_PATHS):
            shutil.rmtree(tmp, ignore_errors=True)
    os._exit(130)


signal.signal(signal.SIGINT, emergency_cleanup)
signal.signal(signal.SIGTERM, emergency_cleanup)

# ============================================================
# 2. Dependencies (rich with self-install)
# ============================================================
try:
    from rich.console import Console
    from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn
    from rich.panel import Panel
    from rich.table import Table
    from rich.syntax import Syntax
    from rich.prompt import Confirm, Prompt
except ImportError:
    print("[*] Installing the 'rich' UI library...")
    subprocess.check_call([sys.executable, "-m", "pip", "install", "rich", "--quiet"])
    from rich.console import Console
    from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn
    from rich.panel import Panel
    from rich.table import Table
    from rich.syntax import Syntax
    from rich.prompt import Confirm, Prompt

console = Console()

# ============================================================
# 3. Paths, constants, basic helpers
# ============================================================
# TWRP/OrangeFox adaptation: this script lives in
# device/google/pixels/sync_tree.py, but operates on the whole Android
# tree root (where .repo lives). CWD-independent: prefer $ANDROID_BUILD_TOP,
# then walk up from CWD and from this file looking for .repo.
def _find_android_root() -> Path:
    env_top = os.environ.get("ANDROID_BUILD_TOP")
    if env_top and Path(env_top).is_dir():
        return Path(env_top).resolve()
    for start in (Path.cwd(), Path(__file__).resolve().parent):
        curr = start.resolve()
        while curr != curr.parent:
            if (curr / ".repo").is_dir() or (curr / "build" / "envsetup.sh").exists():
                return curr
            curr = curr.parent
    return Path(".").resolve()


ROOT_DIR = _find_android_root()
BAK_ROOT = ROOT_DIR / "bakFiles"
ORIG_DIR = BAK_ROOT / "original"
SNAP_ROOT = BAK_ROOT / "snapshots"
NEW_FILES_LIST = BAK_ROOT / "newFiles.txt"
EXCEPTIONS_FILE = BAK_ROOT / "smart_exceptions.json"

# NOTE (TWRP): fox build.sh writes final artifacts to <root>/builds/
# and intermediates to out/target/product/pixels — keep both out of scans.
DEFAULT_EXCEPTIONS = ["out", "builds", "bakFiles", ".repo"]
SKIP_SUFFIXES = (".rej", ".orig", ".patch", ".bak", ".pyc")
STORAGE_BAK_SUFFIXES = (".bp", ".mk")  # files with these extensions are stored with +.bak


def get_target_uid_gid() -> Tuple[int, int]:
    sudo_uid = os.environ.get("SUDO_UID")
    sudo_gid = os.environ.get("SUDO_GID")
    if sudo_uid and sudo_gid:
        return int(sudo_uid), int(sudo_gid)
    return os.getuid(), os.getgid()


def fix_permissions(path: Path, uid: int, gid: int):
    if uid == 0 or not path.exists():
        return
    try:
        subprocess.run(["chown", "-R", f"{uid}:{gid}", str(path)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["chmod", "-R", "u+rwX,go+rX", str(path)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except Exception:
        pass


def to_storage_name(rel: str) -> str:
    """File name inside bakFiles stores (bp/mk files are hidden from the build with a .bak suffix)."""
    return rel + ".bak" if rel.endswith(STORAGE_BAK_SUFFIXES) else rel


def from_storage_name(stored: str) -> str:
    """Reverse mapping: strip .bak only for bp/mk."""
    if stored.endswith(".bak") and stored[:-4].endswith(STORAGE_BAK_SUFFIXES):
        return stored[:-4]
    return stored


def norm_rel(p) -> str:
    return str(p).replace("\\", "/").strip("/")


# ============================================================
# 4. Git helpers
# ============================================================
def get_git_info(file_path: Path) -> Tuple[Optional[Path], Optional[str]]:
    """Returns (git repo root, file path inside it)."""
    resolved = file_path.resolve()
    curr = resolved if resolved.is_dir() else resolved.parent
    while curr != curr.parent:
        if (curr / ".git").exists():
            try:
                return curr, norm_rel(resolved.relative_to(curr))
            except ValueError:
                return curr, None
        curr = curr.parent
    return None, None


def git_show_head(repo_root: Path, rel_in_repo: str) -> Optional[bytes]:
    """Pristine file content from HEAD without touching the working copy."""
    res = subprocess.run(["git", "show", f"HEAD:{rel_in_repo}"],
                         cwd=repo_root, capture_output=True)
    return res.stdout if res.returncode == 0 else None


def git_head_has_file(repo_root: Path, rel_in_repo: str) -> bool:
    res = subprocess.run(["git", "cat-file", "-e", f"HEAD:{rel_in_repo}"],
                         cwd=repo_root, capture_output=True)
    return res.returncode == 0


def git_checkout_head(repo_root: Path, rel_in_repo: str) -> bool:
    """Revert a file to its HEAD state."""
    res = subprocess.run(["git", "checkout", "HEAD", "--", rel_in_repo],
                         cwd=repo_root, capture_output=True)
    return res.returncode == 0


def git_is_modified(repo_root: Path, rel_in_repo: str) -> bool:
    """True if the file differs from HEAD (working copy or index)."""
    res = subprocess.run(["git", "diff", "--quiet", "HEAD", "--", rel_in_repo],
                         cwd=repo_root, capture_output=True)
    return res.returncode == 1


# ============================================================
# 5. Smart-scanner exclusions
# ============================================================
def load_exceptions() -> List[str]:
    if EXCEPTIONS_FILE.exists():
        try:
            data = json.loads(EXCEPTIONS_FILE.read_text(encoding="utf-8"))
            if isinstance(data, list):
                return sorted(set(norm_rel(e) for e in data if str(e).strip()))
        except Exception:
            pass
    return []


def save_exceptions(exceptions: List[str]):
    BAK_ROOT.mkdir(parents=True, exist_ok=True)
    clean = sorted(set(norm_rel(e) for e in exceptions if str(e).strip()))
    EXCEPTIONS_FILE.write_text(json.dumps(clean, indent=2, ensure_ascii=False), encoding="utf-8")


def is_path_excluded(rel_path_str: str, exceptions: List[str]) -> bool:
    norm_path = norm_rel(rel_path_str)
    if norm_path.endswith(SKIP_SUFFIXES):
        return True
    for exc in list(DEFAULT_EXCEPTIONS) + list(exceptions):
        e = norm_rel(exc)
        if e and (norm_path == e or norm_path.startswith(e + "/")):
            return True
    return False


def safe_confirm(prompt_text: str, default: bool = True) -> bool:
    try:
        return Confirm.ask(prompt_text, default=default)
    except Exception:
        try:
            ans = input(f"{prompt_text} [{'Y/n' if default else 'y/N'}]: ").strip().lower()
            if not ans:
                return default
            return ans in ("y", "yes", "1")
        except Exception:
            return default


def manage_exceptions_ui(uid: int, gid: int):
    while True:
        exceptions = load_exceptions()
        console.clear()
        console.print(Panel(
            "[bold cyan]⚙️ Smart-scanner exclusion management (-s/--smart)[/bold cyan]\n"
            "[dim]Listed directories and ALL of their subdirectories are ignored while searching for changes.\n"
            "System exclusions (always): out, bakFiles, .repo[/dim]", expand=False))

        table = Table(title="Current exclusion rules", show_header=True)
        table.add_column("No.", style="cyan", width=5)
        table.add_column("Ignored path (from tree root)", style="bold white")
        if exceptions:
            for idx, exc in enumerate(exceptions, 1):
                table.add_row(str(idx), exc)
        else:
            table.add_row("-", "[italic yellow]No user rules[/italic yellow]")
        console.print(table)

        console.print("\n[bold yellow]Actions:[/bold yellow]")
        console.print("  [bold green]1[/bold green] — Add directory")
        console.print("  [bold red]2[/bold red] — Remove directory")
        console.print("  [bold cyan]0[/bold cyan] — Save and exit\n")
        choice = Prompt.ask("Action", choices=["1", "2", "0"], default="0")

        if choice == "1":
            new_path = norm_rel(Prompt.ask("\n[bold green]Directory path[/bold green]"))
            if new_path:
                if new_path not in exceptions:
                    exceptions.append(new_path)
                    save_exceptions(exceptions)
                    fix_permissions(EXCEPTIONS_FILE, uid, gid)
                    console.print(f"[bold green]✓ Added:[/] {new_path}")
                else:
                    console.print("[yellow]This path is already listed![/yellow]")
            Prompt.ask("\nPress Enter to continue...")
        elif choice == "2":
            if not exceptions:
                console.print("[yellow]The list is empty![/yellow]")
                Prompt.ask("\nPress Enter to continue...")
                continue
            idx_str = Prompt.ask("\n[bold red]Entry number to remove[/bold red]")
            if idx_str.isdigit() and 1 <= int(idx_str) <= len(exceptions):
                removed = exceptions.pop(int(idx_str) - 1)
                save_exceptions(exceptions)
                fix_permissions(EXCEPTIONS_FILE, uid, gid)
                console.print(f"[bold green]✓ Removed:[/] {removed}")
            else:
                console.print("[red]Invalid number![/red]")
            Prompt.ask("\nPress Enter to continue...")
        else:
            break


# ============================================================
# 6. Smart scanner: find changes across all git projects in the tree
# ============================================================
def list_repo_projects() -> Optional[List[str]]:
    """List of tree git projects from the repo manifest (relative paths)."""
    res = subprocess.run(["repo", "list", "-p"], cwd=ROOT_DIR,
                         capture_output=True, text=True)
    if res.returncode != 0:
        return None
    return [norm_rel(line) for line in res.stdout.splitlines() if line.strip()]


def walk_find_git_roots(exceptions: List[str]) -> List[str]:
    """Fallback: walk the tree looking for .git (if repo list is unavailable)."""
    roots = []
    for current_root, dirs, _files in os.walk(ROOT_DIR):
        curr = Path(current_root)
        try:
            rel = norm_rel(curr.relative_to(ROOT_DIR))
        except ValueError:
            rel = norm_rel(curr)
        if rel != "." and is_path_excluded(rel, exceptions):
            dirs[:] = []
            continue
        dirs[:] = [d for d in dirs if d != ".git" and not is_path_excluded(
            d if rel == "." else f"{rel}/{d}", exceptions)]
        if ".git" in os.listdir(current_root):
            roots.append("" if rel == "." else rel)
            dirs[:] = []  # don't look for nested projects inside a project
    return roots


def parse_porcelain_z(data: bytes) -> Tuple[List[str], List[str], List[str]]:
    """Parse `git status --porcelain=v1 -z` -> (modified, untracked, deleted)."""
    modified, untracked, deleted = [], [], []
    fields = data.split(b"\0")
    i = 0
    while i < len(fields):
        field = fields[i]
        i += 1
        if not field:
            continue
        xy = field[:2].decode("ascii", "replace")
        path = os.fsdecode(field[3:])
        if "R" in xy or "C" in xy:
            i += 1  # second field — the old file name
        if xy == "??" or "A" in xy or "R" in xy or "C" in xy:
            untracked.append(path)
        elif "D" in xy:
            deleted.append(path)
        elif any(c in xy for c in ("M", "T", "U")):
            modified.append(path)
    return modified, untracked, deleted


def scan_project(repo_rel: str, exceptions: List[str]):
    """git status of a single project. Returns (repo_rel, modified, untracked, deleted)."""
    repo_abs = ROOT_DIR / repo_rel if repo_rel else ROOT_DIR
    if not (repo_abs / ".git").exists():
        return repo_rel, [], [], []
    res = subprocess.run(
        ["git", "status", "--porcelain=v1", "-z", "--untracked-files=normal"],
        cwd=repo_abs, capture_output=True)
    if res.returncode != 0:
        return repo_rel, [], [], []
    mod, untr, dele = parse_porcelain_z(res.stdout)

    def to_tree(p: str) -> str:
        return norm_rel(f"{repo_rel}/{p}" if repo_rel else p)

    mod = [to_tree(p) for p in mod if not is_path_excluded(to_tree(p), exceptions)]
    untr = [to_tree(p) for p in untr if not is_path_excluded(to_tree(p), exceptions)]
    dele = [to_tree(p) for p in dele if not is_path_excluded(to_tree(p), exceptions)]
    return repo_rel, mod, untr, dele


def smart_scan(exceptions: List[str]) -> Dict[str, Dict[str, List[str]]]:
    """
    Scan the whole tree. Returns a binding to git directories:
    { repo_rel: {"modified": [...], "untracked": [...], "deleted": [...]} }
    (key "" is the root project, if present)
    """
    projects = list_repo_projects()
    if projects is None:
        console.print("[yellow]! repo list is unavailable, falling back to filesystem walk[/yellow]")
        projects = walk_find_git_roots(exceptions)

    projects = [p for p in projects if not is_path_excluded(p, exceptions)]

    result: Dict[str, Dict[str, List[str]]] = {}
    with ThreadPoolExecutor(max_workers=16) as pool:
        futures = [pool.submit(scan_project, p, exceptions) for p in projects]
        for fut in futures:
            repo_rel, mod, untr, dele = fut.result()
            if mod or untr or dele:
                entry = result.setdefault(repo_rel, {"modified": [], "untracked": [], "deleted": []})
                entry["modified"].extend(mod)
                entry["untracked"].extend(untr)
                entry["deleted"].extend(dele)

    for entry in result.values():
        for key in entry:
            entry[key] = sorted(set(entry[key]))
    return dict(sorted(result.items()))


def display_scan_binding(scan: Dict[str, Dict[str, List[str]]]):
    """Table binding the found files to their git directories."""
    table = Table(title="📌 Change-to-git-directory mapping", show_header=True)
    table.add_column("Git directory", style="bold cyan")
    table.add_column("Modified", style="yellow", justify="right")
    table.add_column("New", style="green", justify="right")
    table.add_column("Deleted", style="red", justify="right")
    table.add_column("Files", style="white")
    for repo, data in scan.items():
        files = data["modified"] + data["untracked"]
        shown = "\n".join(files[:8])
        if len(files) > 8:
            shown += f"\n... and {len(files) - 8} more"
        table.add_row(repo or "(tree root)",
                      str(len(data["modified"])),
                      str(len(data["untracked"])),
                      str(len(data["deleted"])),
                      shown)
    console.print(table)


# ============================================================
# 7. Snapshot: back up modified files, revert, originals, patches
# ============================================================
class SnapshotDirs:
    def __init__(self, ts: str):
        self.root = SNAP_ROOT / ts
        self.modified = self.root / "modified"
        self.original = self.root / "original"
        self.patches = self.root / "patches"
        self.new_files = self.root / "new_files"
        self.original_prev = self.root / "original_prev"
        self.manifest = self.root / "manifest.json"

    def mkdirs(self):
        for d in (self.modified, self.original, self.patches):
            d.mkdir(parents=True, exist_ok=True)


def make_patch(orig_file: Path, modified_file: Path, patch_path: Path, rel: str) -> bool:
    """Unified diff orig -> modified with a/<rel>, b/<rel> labels. True if the patch is non-empty."""
    res = subprocess.run(
        ["diff", "-u", "--label", f"a/{rel}", "--label", f"b/{rel}",
         str(orig_file), str(modified_file)],
        capture_output=True)
    if res.returncode == 1 and res.stdout:
        patch_path.parent.mkdir(parents=True, exist_ok=True)
        patch_path.write_bytes(res.stdout)
        return True
    return False


def process_modified_file(rel: str, repo_abs: Path, snap: SnapshotDirs,
                          uid: int, gid: int) -> Tuple[str, str]:
    """
    Pipeline for a single modified file:
    back up current -> revert to HEAD -> copy original -> generate patch.
    """
    tree_file = ROOT_DIR / rel
    storage = to_storage_name(rel)
    snap_mod = snap.modified / storage
    snap_orig = snap.original / storage
    live_orig = ORIG_DIR / storage
    patch_path = snap.patches / f"{rel}.patch"

    for p in (snap_mod, snap_orig, live_orig):
        p.parent.mkdir(parents=True, exist_ok=True)

    if not tree_file.exists():
        return "missing", f"File missing from the tree: {rel}"

    # 1. Back up the CURRENT (modified) file
    shutil.copy2(tree_file, snap_mod)

    # 2. Revert changes to HEAD
    rel_in_repo = norm_rel(tree_file.resolve().relative_to(repo_abs))
    if not git_checkout_head(repo_abs, rel_in_repo):
        head_data = git_show_head(repo_abs, rel_in_repo)
        if head_data is None:
            return "revert_failed", f"Failed to revert to HEAD: {rel}"
        tree_file.write_bytes(head_data)

    # 3. Pristine original -> into the snapshot and the live original/ store
    shutil.copy2(tree_file, snap_orig)
    shutil.copy2(tree_file, live_orig)
    fix_permissions(live_orig, uid, gid)

    # 4. Patch: original vs the saved modified copy
    if make_patch(snap_orig, snap_mod, patch_path, rel):
        return "ok", f"Snapshot ready: {rel}"
    return "nodiff", f"No differences from HEAD found: {rel}"


def clean_original_dir(snap: SnapshotDirs):
    """Smart mode: wipe original/ backing up the previous content."""
    if ORIG_DIR.exists() and any(ORIG_DIR.iterdir()):
        snap.original_prev.mkdir(parents=True, exist_ok=True)
        for item in ORIG_DIR.iterdir():
            shutil.move(str(item), str(snap.original_prev / item.name))
    ORIG_DIR.mkdir(parents=True, exist_ok=True)


def move_new_files_aside(new_files: List[str], snap: SnapshotDirs, uid: int, gid: int):
    """Temporarily move untracked files out of the tree into the snapshot."""
    if not new_files:
        return
    snap.new_files.mkdir(parents=True, exist_ok=True)
    for rel in new_files:
        src = ROOT_DIR / rel
        dest = snap.new_files / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dest, dirs_exist_ok=True)
            shutil.rmtree(src, ignore_errors=True)
        elif src.exists():
            shutil.copy2(src, dest)
            src.unlink()


def restore_new_files(snap: SnapshotDirs, uid: int, gid: int) -> int:
    """Restore untracked files from the snapshot back into the tree."""
    if not snap.new_files.exists():
        return 0
    count = 0
    for src in snap.new_files.rglob("*"):
        if src.is_file():
            target = ROOT_DIR / src.relative_to(snap.new_files)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, target)
            fix_permissions(target, uid, gid)
            count += 1
    return count


def refresh_original_from_head(rel: str, uid: int, gid: int) -> bool:
    """Refresh bakFiles/original with the file content from the CURRENT HEAD (after sync)."""
    repo_root, rel_in_repo = get_git_info(ROOT_DIR / rel)
    if not repo_root or not rel_in_repo:
        return False
    data = git_show_head(repo_root, rel_in_repo)
    if data is None:
        return False
    live_orig = ORIG_DIR / to_storage_name(rel)
    live_orig.parent.mkdir(parents=True, exist_ok=True)
    live_orig.write_bytes(data)
    fix_permissions(live_orig, uid, gid)
    return True


# ============================================================
# 8. Resilient patch-application engine
# ============================================================
def _run_patch(args: List[str], target: Path, patch_bytes: bytes) -> subprocess.CompletedProcess:
    return subprocess.run(["patch"] + args + [str(target)],
                          input=patch_bytes, capture_output=True)


def apply_patch_smart(rel: str, patch_path: Path, orig_file: Path,
                      snap_mod: Optional[Path], uid: int, gid: int) -> Tuple[str, str]:
    """
    Attempt order:
      1) do the target/patch exist?
      2) are the changes already present? (reverse dry-run)
      3) patch -l --fuzz=3 (whitespace tolerant), with a safety net
      4) fallback: three-way git merge-file (tree / original / snapshot copy)
      5) failed with a detailed report (the target file is left untouched)
    """
    target = ROOT_DIR / rel
    if not target.exists():
        return "missing", f"Target file not found: {rel}"
    if not patch_path.exists() or patch_path.stat().st_size == 0:
        return "skipped", f"Patch empty or missing: {rel}"

    patch_bytes = patch_path.read_bytes()
    repo_root, rel_in_repo = get_git_info(target)

    # --- 2. Already applied? ---
    chk = _run_patch(["-p0", "-R", "--dry-run", "-s", "-l", "--fuzz=3"], target, patch_bytes)
    if chk.returncode == 0:
        return "already_applied", f"Changes already present (skipping): {rel}"

    # --- 3. Direct apply with a safety net ---
    tmp_dir = Path(tempfile.mkdtemp(prefix="sync_tree_patch_"))
    ACTIVE_TEMP_PATHS.add(tmp_dir)
    try:
        backup = tmp_dir / "target.orig"
        shutil.copy2(target, backup)
        res = _run_patch(["-p0", "-N", "-s", "-l", "--fuzz=3", "--no-backup-if-mismatch"],
                         target, patch_bytes)
        if res.returncode == 0:
            fix_permissions(target, uid, gid)
            for junk in (Path(f"{target}.rej"), Path(f"{target}.orig")):
                junk.unlink(missing_ok=True)
            return "applied", f"Patch applied: {rel}"
        # Failure -> restore the target file
        shutil.copy2(backup, target)
        for junk in (Path(f"{target}.rej"), Path(f"{target}.orig")):
            junk.unlink(missing_ok=True)

        # --- 4. Fallback: 3-way merge ---
        if orig_file.exists() and snap_mod and snap_mod.exists():
            merge = subprocess.run(
                ["git", "merge-file", "-p",
                 "-L", f"current:{rel}", "-L", f"base:{rel}", "-L", f"local-changes:{rel}",
                 str(target), str(orig_file), str(snap_mod)],
                capture_output=True)
            if merge.returncode == 0 and merge.stdout:
                target.write_bytes(merge.stdout)
                fix_permissions(target, uid, gid)
                return "merged", f"Applied via 3-way merge (context drifted): {rel}"
            return "failed", (f"3-way merge conflict: {rel} "
                              f"(needs manual resolution; patch: {patch_path})")
        return "failed", f"Patch does not apply and no 3-way merge data: {rel}"
    finally:
        ACTIVE_TEMP_PATHS.discard(tmp_dir)
        shutil.rmtree(tmp_dir, ignore_errors=True)


def apply_patches_from_dir(patches_dir: Path, snap: Optional[SnapshotDirs],
                           uid: int, gid: int,
                           only: Optional[List[str]] = None) -> Dict[str, List[Tuple[str, str]]]:
    """Apply all *.patch from a directory. Returns {status: [(rel, msg)]}."""
    results: Dict[str, List[Tuple[str, str]]] = {}
    patch_files = sorted(patches_dir.rglob("*.patch")) if patches_dir.exists() else []
    if not patch_files:
        console.print("[yellow]No patches to apply.[/yellow]")
        return results

    for patch_file in patch_files:
        rel = norm_rel(patch_file.relative_to(patches_dir))[:-6]  # strip ".patch"
        if only and rel not in only:
            continue
        storage = to_storage_name(rel)
        if snap is not None:
            orig_file = snap.original / storage
            if not orig_file.exists():
                orig_file = snap.original / rel  # legacy names without .bak
            if not orig_file.exists():
                orig_file = ORIG_DIR / storage
            if not orig_file.exists():
                orig_file = ORIG_DIR / rel
            snap_mod = snap.modified / storage
            if not snap_mod.exists():
                snap_mod = snap.modified / rel  # legacy names without .bak
            if not snap_mod.exists():
                snap_mod = None
        else:
            orig_file = ORIG_DIR / storage
            snap_mod = None

        status, msg = apply_patch_smart(rel, patch_file, orig_file, snap_mod, uid, gid)
        results.setdefault(status, []).append((rel, msg))
        color = "bold green" if status in ("applied", "already_applied", "merged") else \
                "yellow" if status in ("skipped", "missing") else "bold red"
        console.print(f"  [{color}]{msg}[/]")

        # After a successful apply, refresh the live original to the new HEAD
        if status in ("applied", "merged", "already_applied"):
            refresh_original_from_head(rel, uid, gid)
    return results


def print_apply_summary(results: Dict[str, List[Tuple[str, str]]]):
    table = Table(title="📊 Patch apply summary", show_header=True)
    table.add_column("Status", style="bold")
    table.add_column("Files", justify="right")
    table.add_column("List", style="dim")
    labels = {
        "applied": ("✅ Applied", "green"),
        "already_applied": ("✓ Already applied", "green"),
        "merged": ("🔀 Applied via 3-way merge", "cyan"),
        "skipped": ("⏭ Skipped (empty patch)", "yellow"),
        "missing": ("❓ Target file missing", "yellow"),
        "failed": ("❌ Failed", "red"),
    }
    total_fail = 0
    for status, items in results.items():
        label, color = labels.get(status, (status, "white"))
        names = "\n".join(rel for rel, _ in items[:6])
        if len(items) > 6:
            names += f"\n... and {len(items) - 6} more"
        table.add_row(f"[{color}]{label}[/]", str(len(items)), names)
        if status == "failed":
            total_fail = len(items)
    console.print(table)
    return total_fail


# ============================================================
# 9. repo sync
# ============================================================
def run_repo_sync(jobs: int, force: bool = False) -> int:
    cmd = ["repo", "sync", "-c", "--force-sync", "--no-clone-bundle", "--no-tags"]
    if not force:
        cmd.append("--prune")
    cmd.append(f"-j{jobs}")
    console.print(f"\n[bold cyan]>>> {' '.join(cmd)}[/bold cyan]")
    return subprocess.run(cmd, cwd=ROOT_DIR).returncode


# ============================================================
# 9b. OrangeFox sync flow (official OFOX update scheme)
# ============================================================
# Mirrors OrangeFox sync/update_fox.sh and the OFOX wiki:
#   repo sync            (android_bootable_recovery errors are EXPECTED
#                         and must be ignored)
#   git pull             bootable/recovery
#   git pull             vendor/recovery (+ external/se_omapi if present)
# In OFOX trees bootable/recovery is a manual git clone of the OFOX
# Recovery repo (real .git dir, e.g. branch fox_14.1), while the repo
# manifest declares nebrassy/android_bootable_recovery@android-14.
# `repo sync` therefore always fails that project with
# "unsupported checkout state". We implement the documented "ignore"
# deterministically: repo-sync every manifest project EXCEPT the manual
# OFOX clones, then `git pull` the manual clones (official order).
OFOX_CONFIG_FILE = BAK_ROOT / "ofox_sync.json"
OFOX_SUPPORTED_BRANCHES = ["14.1", "12.1"]
OFOX_SYNC_REPO_HTTPS = "https://gitlab.com/OrangeFox/sync.git"
OFOX_SYNC_REPO_SSH = "git@gitlab.com:OrangeFox/sync.git"
# Manifest paths that are manual OFOX git clones, not repo checkouts.
OFOX_MANUAL_PROJECTS = ["bootable/recovery"]
# Tree-relative dirs updated via `git pull`, in official order.
OFOX_GIT_PULL_REQUIRED = ["bootable/recovery"]
OFOX_GIT_PULL_OPTIONAL = ["vendor/recovery", "external/se_omapi"]


def load_ofox_config() -> dict:
    cfg = {"sync_tools_dir": None, "branch": None, "use_ofox_flow": False}
    if OFOX_CONFIG_FILE.exists():
        try:
            data = json.loads(OFOX_CONFIG_FILE.read_text(encoding="utf-8"))
            if isinstance(data, dict):
                for key in cfg:
                    if data.get(key) is not None:
                        cfg[key] = data[key]
        except Exception:
            pass
    return cfg


def save_ofox_config(cfg: dict, uid: int, gid: int):
    BAK_ROOT.mkdir(parents=True, exist_ok=True)
    clean = {
        "sync_tools_dir": cfg.get("sync_tools_dir"),
        "branch": cfg.get("branch"),
        "use_ofox_flow": bool(cfg.get("use_ofox_flow", False)),
    }
    OFOX_CONFIG_FILE.write_text(json.dumps(clean, indent=2, ensure_ascii=False), encoding="utf-8")
    fix_permissions(OFOX_CONFIG_FILE, uid, gid)


def detect_ofox_branch() -> Optional[str]:
    """Guess the OFOX branch (14.1/12.1) from the bootable/recovery git branch."""
    rec = ROOT_DIR / "bootable" / "recovery"
    if not (rec / ".git").exists():
        return None
    res = subprocess.run(["git", "branch", "--show-current"],
                         cwd=rec, capture_output=True, text=True)
    if res.returncode == 0:
        branch = res.stdout.strip()
        for supported in OFOX_SUPPORTED_BRANCHES:
            if supported in branch:
                return supported
    return None


def default_ofox_tools_dir() -> Optional[Path]:
    """Sibling OrangeFox_sync/sync next to the tree root, if already cloned."""
    cand = ROOT_DIR.parent / "OrangeFox_sync" / "sync"
    if (cand / "orangefox_sync.sh").exists():
        return cand
    return None


def ensure_ofox_tools_dir(cfg: dict, uid: int, gid: int, interactive: bool) -> Optional[Path]:
    """Resolve the OFOX sync-tools dir: config -> sibling default -> ask/clone.

    Never fatal: the update flow (repo sync + git pulls) does not need the
    tools dir — it is only required for a first-time `orangefox_sync.sh` fetch.
    """
    configured = cfg.get("sync_tools_dir")
    if configured and (Path(configured) / "orangefox_sync.sh").exists():
        return Path(configured)
    default = default_ofox_tools_dir()
    if default is not None:
        cfg["sync_tools_dir"] = str(default)
        save_ofox_config(cfg, uid, gid)
        return default
    if not interactive:
        console.print("[yellow]OFOX sync tools not found; continuing without them "
                      "(only a first-time orangefox_sync.sh fetch needs them).[/yellow]")
        return None
    try:
        answer = Prompt.ask("OrangeFox sync tools dir (empty = skip)", default="").strip()
    except Exception:
        return None
    if not answer:
        return None
    tools = Path(answer).expanduser()
    if not (tools / "orangefox_sync.sh").exists():
        use_ssh = os.environ.get("USE_SSH", "0") == "1"
        url = OFOX_SYNC_REPO_SSH if use_ssh else OFOX_SYNC_REPO_HTTPS
        if safe_confirm(f"Clone OFOX sync tools from {url} into {tools}?", True):
            tools.parent.mkdir(parents=True, exist_ok=True)
            rc = subprocess.run(["git", "clone", url, str(tools)]).returncode
            if rc != 0 or not (tools / "orangefox_sync.sh").exists():
                console.print("[bold red]❌ Failed to clone OFOX sync tools.[/bold red]")
                return None
        else:
            return None
    cfg["sync_tools_dir"] = str(tools)
    save_ofox_config(cfg, uid, gid)
    return tools


def ofox_setup_ui(cfg: dict, uid: int, gid: int):
    console.print(Panel(
        "[bold cyan]🦊 OrangeFox sync configuration[/bold cyan]\n"
        "[dim]Official scheme: repo sync (android_bootable_recovery errors are expected\n"
        "and ignored) + git pull in bootable/recovery and vendor/recovery.[/dim]",
        expand=False))
    console.print(f"  Current tools dir: [white]{cfg.get('sync_tools_dir') or '-'}[/white]")
    console.print(f"  Current branch:    [white]{cfg.get('branch') or '-'}[/white]")
    console.print(f"  OFOX flow default: [white]{'on' if cfg.get('use_ofox_flow') else 'off'}[/white]\n")

    ensure_ofox_tools_dir(cfg, uid, gid, interactive=True)

    detected = detect_ofox_branch()
    current = cfg.get("branch") or detected or "14.1"
    if current not in OFOX_SUPPORTED_BRANCHES:
        current = "14.1"
    try:
        branch = Prompt.ask("OFOX branch", choices=OFOX_SUPPORTED_BRANCHES,
                            default=current)
    except Exception:
        branch = current
    cfg["branch"] = branch

    try:
        use_flow = Confirm.ask("Use OFOX sync flow by default?", default=True)
    except Exception:
        use_flow = True
    cfg["use_ofox_flow"] = use_flow
    save_ofox_config(cfg, uid, gid)
    console.print(f"[green]✓ OFOX config saved:[/] {OFOX_CONFIG_FILE}")


def run_ofox_sync(jobs: int, force: bool = False) -> int:
    """OFOX update flow: repo-sync managed projects, then git-pull manual clones."""
    projects = list_repo_projects()
    base = ["repo", "sync", "-c", "--force-sync", "--no-clone-bundle", "--no-tags"]
    if not force:
        base.append("--prune")
    if projects is None:
        console.print("[yellow]! repo list unavailable — running plain repo sync; "
                      "android_bootable_recovery errors are expected per OFOX docs.[/yellow]")
        cmd = base + [f"-j{jobs}"]
        console.print(f"\n[bold cyan]>>> {' '.join(cmd)}[/bold cyan]")
        rc = subprocess.run(cmd, cwd=ROOT_DIR).returncode
        if rc != 0:
            console.print("[yellow]Continuing with OFOX git pulls despite repo sync rc "
                          f"{rc} (recovery checkout errors are expected).[/yellow]")
    else:
        targets = [p for p in projects if p not in OFOX_MANUAL_PROJECTS]
        skipped = [p for p in projects if p in OFOX_MANUAL_PROJECTS]
        if skipped:
            console.print("[yellow]OFOX flow: skipping manual clone(s): "
                          f"{', '.join(skipped)} (updated via git pull below).[/yellow]")
        if not targets:
            console.print("[yellow]No managed projects to sync; going straight to git pulls.[/yellow]")
        else:
            console.print(f"\n[bold cyan]>>> repo sync ({len(targets)} projects, -j{jobs}, "
                          "recovery excluded)[/bold cyan]")
            rc = subprocess.run(base + [f"-j{jobs}"] + targets, cwd=ROOT_DIR).returncode
            if rc != 0:
                console.print("\n[bold red]❌ repo sync failed on managed projects![/bold red]")
                return rc
    for rel in OFOX_GIT_PULL_REQUIRED + OFOX_GIT_PULL_OPTIONAL:
        target = ROOT_DIR / rel
        if not target.is_dir():
            if rel in OFOX_GIT_PULL_REQUIRED:
                console.print(f"[bold red]❌ Required dir missing: {rel}[/bold red]")
                return 1
            continue
        if not (target / ".git").exists():
            console.print(f"[yellow]Skipping git pull: {rel} is not a git checkout.[/yellow]")
            continue
        console.print(f"\n[bold cyan]>>> git pull ({rel})[/bold cyan]")
        rc = subprocess.run(["git", "pull"], cwd=target).returncode
        if rc != 0:
            console.print(f"[bold red]❌ git pull failed in {rel}[/bold red]")
            return rc
    return 0


def do_tree_sync(args, uid: int, gid: int, interactive: bool) -> int:
    """Dispatch plain repo sync vs the OFOX flow based on flags/config."""
    cfg = load_ofox_config()
    if getattr(args, "ofox_tools", None):
        cfg["sync_tools_dir"] = str(Path(getattr(args, "ofox_tools")).expanduser())
        save_ofox_config(cfg, uid, gid)
    if getattr(args, "ofox_branch", None):
        cfg["branch"] = getattr(args, "ofox_branch")
        save_ofox_config(cfg, uid, gid)
    use_ofox = bool(getattr(args, "ofox_sync", False) or cfg.get("use_ofox_flow"))
    if use_ofox:
        if not cfg.get("branch"):
            cfg["branch"] = detect_ofox_branch() or "14.1"
            save_ofox_config(cfg, uid, gid)
        ensure_ofox_tools_dir(cfg, uid, gid, interactive)
        console.print(Panel(f"[bold cyan]🦊 OrangeFox sync flow[/bold cyan] "
                            f"[dim](branch {cfg.get('branch')})[/dim]", expand=False))
        return run_ofox_sync(args.jobs, force=args.forcesync)
    return run_repo_sync(args.jobs, force=args.forcesync)


# ============================================================
# 10. Interactive snapshot manager (--change / -c)
# ============================================================
def _snapshot_patch_files(snap_dir: Path) -> List[Path]:
    """Snapshot patches: new layout (patches/) and legacy (diff/)."""
    for sub in ("patches", "diff"):
        d = snap_dir / sub
        if d.exists():
            files = sorted(d.rglob("*.patch"))
            if files:
                return files
    return []


def _snapshot_dirs_compat(snap_dir: Path) -> SnapshotDirs:
    """SnapshotDirs with legacy name substitution (copy/ -> modified/, diff/ -> patches/)."""
    snap = SnapshotDirs(snap_dir.name)
    snap.root = snap_dir
    snap.modified = snap_dir / "modified" if (snap_dir / "modified").exists() else snap_dir / "copy"
    snap.original = snap_dir / "original"
    snap.patches = snap_dir / "patches" if (snap_dir / "patches").exists() else snap_dir / "diff"
    snap.new_files = snap_dir / "new_files" if (snap_dir / "new_files").exists() else snap_dir / "new_files_temp"
    return snap


def change_mode_ui(uid: int, gid: int):
    console.print(Panel("[bold cyan]🗂 Snapshot and patch manager (--change)[/bold cyan]\n"
                        "[dim]Pick a snapshot → view/apply individual patches or all at once.[/dim]",
                        expand=False))
    if not SNAP_ROOT.exists():
        console.print("[bold red]❌ Snapshot directory not found (bakFiles/snapshots).[/bold red]")
        return

    while True:
        snaps = sorted([d for d in SNAP_ROOT.iterdir() if d.is_dir()],
                       key=lambda d: d.name, reverse=True)
        if not snaps:
            console.print("[bold red]❌ No snapshots found.[/bold red]")
            return

        table = Table(title="Available snapshots", show_header=True)
        table.add_column("No.", style="cyan", width=5)
        table.add_column("Snapshot", style="bold white")
        table.add_column("Patches", justify="right", style="yellow")
        for idx, s in enumerate(snaps, 1):
            table.add_row(str(idx), s.name, str(len(_snapshot_patch_files(s))))
        console.print(table)

        choice = Prompt.ask("Snapshot number (q to quit)", default="q")
        if choice.lower() == "q":
            return
        if not choice.isdigit() or not (1 <= int(choice) <= len(snaps)):
            console.print("[red]Invalid number![/red]")
            continue

        snap_dir = snaps[int(choice) - 1]
        snap = _snapshot_dirs_compat(snap_dir)
        patch_files = _snapshot_patch_files(snap_dir)
        if not patch_files:
            console.print("[yellow]This snapshot has no patches.[/yellow]")
            continue

        while True:
            table = Table(title=f"Files in snapshot {snap_dir.name}", show_header=True)
            table.add_column("No.", style="cyan", width=5)
            table.add_column("Target file", style="white")
            table.add_column("Size", justify="right", style="dim")
            rels = []
            for idx, pf in enumerate(patch_files, 1):
                rel = norm_rel(pf.relative_to(snap.patches))[:-6]
                rels.append(rel)
                table.add_row(str(idx), rel, f"{pf.stat().st_size} B")
            console.print(table)
            console.print("[bold yellow]Commands:[/bold yellow] number — view/apply, "
                          "[bold green]a[/bold green] — apply ALL snapshot patches, q — back")
            cmd = Prompt.ask("Command", default="q")

            if cmd.lower() == "q":
                break
            if cmd.lower() == "a":
                if safe_confirm(f"Apply ALL {len(patch_files)} patches from {snap_dir.name}?", True):
                    results = apply_patches_from_dir(snap.patches, snap, uid, gid)
                    print_apply_summary(results)
                    Prompt.ask("\nPress Enter to continue...")
                continue
            if not cmd.isdigit() or not (1 <= int(cmd) <= len(patch_files)):
                console.print("[red]Invalid number![/red]")
                continue

            pf = patch_files[int(cmd) - 1]
            rel = rels[int(cmd) - 1]
            syntax = Syntax(pf.read_text(errors="replace"), "diff",
                            theme="monokai", line_numbers=True)
            console.print(Panel(syntax, title=f"Patch: {rel}", border_style="cyan", expand=True))
            if safe_confirm(f"Apply this patch to [cyan]{rel}[/cyan]?", True):
                results = apply_patches_from_dir(snap.patches, snap, uid, gid, only=[rel])
                print_apply_summary(results)
                Prompt.ask("\nPress Enter to continue...")


# ============================================================
# 11. Change-list collection
# ============================================================
def gather_from_original(exceptions: List[str]) -> Dict[str, Dict[str, List[str]]]:
    """
    Default mode: candidates = files from bakFiles/original.
    A local modification is confirmed via git diff HEAD (protection against
    upstream changes: if only the repo changed the file — it is not a local patch).
    """
    result: Dict[str, Dict[str, List[str]]] = {}
    if not ORIG_DIR.exists():
        return result
    for stored in sorted(ORIG_DIR.rglob("*")):
        if not stored.is_file():
            continue
        rel = from_storage_name(norm_rel(stored.relative_to(ORIG_DIR)))
        if is_path_excluded(rel, exceptions):
            continue
        tree_file = ROOT_DIR / rel
        if not tree_file.exists():
            continue
        repo_root, rel_in_repo = get_git_info(tree_file)
        if not repo_root or not rel_in_repo:
            continue
        if not git_head_has_file(repo_root, rel_in_repo):
            continue
        if not git_is_modified(repo_root, rel_in_repo):
            continue  # tree is clean — no local changes
        repo_rel = norm_rel(repo_root.relative_to(ROOT_DIR)) \
            if repo_root != ROOT_DIR else ""
        entry = result.setdefault(repo_rel, {"modified": [], "untracked": [], "deleted": []})
        entry["modified"].append(rel)

    # Compatibility: extra new files from newFiles.txt
    if NEW_FILES_LIST.exists():
        for line in NEW_FILES_LIST.read_text().splitlines():
            line = norm_rel(line)
            if not line or line.startswith("#"):
                continue
            p = ROOT_DIR / line
            if p.exists() and not is_path_excluded(line, exceptions):
                repo_root, _ = get_git_info(p)
                repo_rel = norm_rel(repo_root.relative_to(ROOT_DIR)) if repo_root and repo_root != ROOT_DIR else ""
                entry = result.setdefault(repo_rel, {"modified": [], "untracked": [], "deleted": []})
                if line not in entry["untracked"]:
                    entry["untracked"].append(line)
    return dict(sorted(result.items()))


def flatten_modified(scan: Dict[str, Dict[str, List[str]]]) -> List[str]:
    out = []
    for data in scan.values():
        out.extend(data["modified"])
    return sorted(set(out))


def flatten_untracked(scan: Dict[str, Dict[str, List[str]]]) -> List[str]:
    out = []
    for data in scan.values():
        out.extend(data["untracked"])
    return sorted(set(out))


def write_manifest(snap: SnapshotDirs, mode: str,
                   scan: Dict[str, Dict[str, List[str]]],
                   processed: Dict[str, str], extra: Optional[dict] = None):
    manifest = {
        "timestamp": snap.root.name,
        "created": datetime.now().isoformat(),
        "mode": mode,
        "repos": scan,
        "processed": processed,
    }
    if extra:
        manifest.update(extra)
    snap.manifest.write_text(json.dumps(manifest, indent=2, ensure_ascii=False),
                             encoding="utf-8")


# ============================================================
# 12. Main pipeline
# ============================================================
def snapshot_pipeline(scan: Dict[str, Dict[str, List[str]]], snap: SnapshotDirs,
                      uid: int, gid: int, clean_original: bool,
                      interactive: bool) -> Tuple[List[str], Dict[str, str]]:
    """
    Confirm -> back up -> revert -> originals -> patches -> remove new files.
    Returns (new file list, {rel: processing status}).
    """
    modified = flatten_modified(scan)
    untracked = flatten_untracked(scan)
    deleted = flatten_modified({r: {"modified": d["deleted"]} for r, d in scan.items()})

    if deleted:
        console.print("[yellow]⚠ Files deleted from the tree found (skipping, backup is impossible):[/yellow]")
        for d in deleted:
            console.print(f"  [dim]{d}[/dim]")

    if interactive:
        if modified and not safe_confirm(
                f"Save and prepare patches for ALL {len(modified)} modified files?", True):
            keep = []
            for f in modified:
                if safe_confirm(f"  Include [cyan]{f}[/cyan]?", True):
                    keep.append(f)
            modified = keep
            scan = {r: {"modified": [f for f in d["modified"] if f in keep],
                        "untracked": d["untracked"], "deleted": d["deleted"]}
                    for r, d in scan.items()}
        if untracked and not safe_confirm(
                f"Temporarily remove and save ALL {len(untracked)} new/untracked files?", True):
            keep_u = []
            for f in untracked:
                if safe_confirm(f"  Include [cyan]{f}[/cyan]?", True):
                    keep_u.append(f)
            untracked = keep_u

    if clean_original:
        console.print("[yellow]--> Wiping original/ (previous content goes to the snapshot's original_prev/)[/yellow]")
        clean_original_dir(snap)
    ORIG_DIR.mkdir(parents=True, exist_ok=True)

    processed: Dict[str, str] = {}
    for repo_rel, data in scan.items():
        repo_abs = ROOT_DIR / repo_rel if repo_rel else ROOT_DIR
        for rel in data["modified"]:
            status, msg = process_modified_file(rel, repo_abs, snap, uid, gid)
            processed[rel] = status
            color = "green" if status == "ok" else "yellow" if status == "nodiff" else "red"
            console.print(f"  [{color}]{msg}[/]")

    if untracked:
        console.print(f"[yellow]--> Temporarily removing {len(untracked)} new/untracked files[/yellow]")
        move_new_files_aside(untracked, snap, uid, gid)

    return untracked, processed


def restore_modified_from_snapshot(snap: SnapshotDirs, uid: int, gid: int) -> int:
    """Restore modified files from the snapshot back into the tree (-d mode, rollback on sync failure)."""
    count = 0
    if not snap.modified.exists():
        return 0
    for src in snap.modified.rglob("*"):
        if not src.is_file():
            continue
        rel = from_storage_name(norm_rel(src.relative_to(snap.modified)))
        target = ROOT_DIR / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, target)
        fix_permissions(target, uid, gid)
        count += 1
    return count


# ============================================================
# 13. Entry point
# ============================================================
def main():
    parser = argparse.ArgumentParser(
        description="Smart sync of the Android source tree preserving local patches")
    parser.add_argument("-s", "--smart", action="store_true",
                        help="Deep smart scanner of git repos to find changes")
    parser.add_argument("-d", "--diff", action="store_true",
                        help="(with -s) Only prepare a snapshot of the changes, no repo sync")
    parser.add_argument("-c", "--change", action="store_true",
                        help="Snapshot picker dialog: view and apply patches")
    parser.add_argument("-f", "--forcesync", action="store_true",
                        help="Full force sync with revert of changes, without applying patches "
                             "(a snapshot is always taken; without -s — from bakFiles/original, "
                             "with -s — deep scan)")
    parser.add_argument("-r", "--revert", action="store_true",
                        help="Only revert local changes to HEAD (with snapshot), no repo sync. "
                             "Without -s — from bakFiles/original, with -s — deep scan")
    parser.add_argument("-e", "--except", dest="manage_exceptions", action="store_true",
                        help="Open the smart-scanner exclusion management UI")
    parser.add_argument("-j", "--jobs", type=int, default=os.cpu_count() or 8,
                        help="repo sync thread count")
    parser.add_argument("-y", "--yes", action="store_true",
                        help="Non-interactive mode (auto-confirm)")
    parser.add_argument("--ofox-sync", dest="ofox_sync", action="store_true",
                        help="Use the official OrangeFox update flow: repo sync all projects "
                             "except manual OFOX clones (bootable/recovery), then git pull "
                             "bootable/recovery + vendor/recovery")
    parser.add_argument("--ofox-setup", dest="ofox_setup", action="store_true",
                        help="Configure OFOX sync (tools dir, branch, default flow). "
                             "Exits unless combined with a sync mode")
    parser.add_argument("--ofox-tools", type=Path, default=None,
                        help="Path to OrangeFox sync tools dir (with orangefox_sync.sh); saved to config")
    parser.add_argument("--ofox-branch", choices=["14.1", "12.1"], default=None,
                        help="OFOX manifest branch; saved to config")
    args = parser.parse_args()

    uid, gid = get_target_uid_gid()
    interactive = not args.yes

    console.print(Panel("[bold cyan]🔄 Smart sync of the Android source tree[/bold cyan]",
                        expand=False))

    # --- Exclusion UI ---
    if args.manage_exceptions:
        manage_exceptions_ui(uid, gid)
        if not (args.smart or args.forcesync or args.change or args.revert):
            return

    # --- OFOX setup UI ---
    if args.ofox_setup:
        ofox_setup_ui(load_ofox_config(), uid, gid)
        if not (args.smart or args.forcesync or args.change or args.revert):
            return

    # --- Snapshot manager ---
    if args.change:
        change_mode_ui(uid, gid)
        return

    if args.diff and not args.smart:
        console.print("[bold red]❌ The -d/--diff flag only works together with -s/--smart![/bold red]")
        sys.exit(2)

    if args.revert and args.forcesync:
        console.print("[bold red]❌ Flags -r/--revert and -f/--forcesync are incompatible "
                      "(revert is without sync, forcesync is with sync).[/bold red]")
        sys.exit(2)
    if args.revert and args.diff:
        console.print("[bold red]❌ Flags -r/--revert and -d/--diff are incompatible "
                      "(diff restores files to the tree, revert rolls them back).[/bold red]")
        sys.exit(2)

    # --- FORCE SYNC goes through the shared pipeline so that changes are
    # --- found (via original/ or the deep -s scan), saved to a snapshot,
    # --- reverted to HEAD and NOT applied back after sync.
    # --- There is no early "bare" repo sync without a snapshot anymore.
    is_force = args.forcesync
    is_revert = args.revert
    if is_force:
        console.print("[bold yellow]! FORCE SYNC mode: changes will be saved "
                      "to a snapshot, reverted to HEAD and NOT applied back[/bold yellow]")
    if is_revert:
        console.print("[bold yellow]! REVERT mode: revert changes to HEAD "
                      "without repo sync (the snapshot is kept, patches will NOT be restored)[/bold yellow]")

    # --- Collect changes ---
    exceptions = load_exceptions()
    PROGRESS_COLUMNS = [SpinnerColumn(), TextColumn("{task.description}"),
                        BarColumn(bar_width=30), TextColumn("[progress.percentage]{task.percentage:>3.0f}%")]

    with Progress(*PROGRESS_COLUMNS, console=console, transient=True) as progress:
        task = progress.add_task("[cyan]Searching the tree for changes...", total=None)
        if args.smart:
            scan = smart_scan(exceptions)
        else:
            scan = gather_from_original(exceptions)

    modified = flatten_modified(scan)
    untracked = flatten_untracked(scan)

    if not scan or (not modified and not untracked):
        # -f / -r without -s only look at bakFiles/original and may miss
        # edits outside the tracked list — offer a deep scan.
        if (is_force or is_revert) and not args.smart:
            console.print("[yellow]ℹ No changes found in bakFiles/original, "
                          "but that does not guarantee a clean tree.[/yellow]")
            do_smart = False
            if interactive:
                action = "REVERT" if is_revert else "FORCE SYNC"
                do_smart = safe_confirm(
                    f"Run a deep smart scan of all git projects before {action}?", True)
            if do_smart:
                with Progress(*PROGRESS_COLUMNS, console=console, transient=True) as progress2:
                    task2 = progress2.add_task("[cyan]Deep change search (-s)...", total=None)
                    scan = smart_scan(exceptions)
                modified = flatten_modified(scan)
                untracked = flatten_untracked(scan)
                if scan and (modified or untracked):
                    console.print("[bold yellow]--> Deep scan found changes, "
                                  "moving on to snapshot and revert.[/bold yellow]")
                    # the shared display below will show the table, skipping the exit
                else:
                    console.print("[green]✓ Deep scan found nothing either.[/green]")
                    if is_revert:
                        console.print("[green]Nothing to revert — the tree is clean.[/green]")
                        return
                    if args.diff:
                        console.print("[yellow]Snapshot is empty — exiting without sync.[/yellow]")
                        return
                    sys.exit(do_tree_sync(args, uid, gid, interactive))
            else:
                if not ORIG_DIR.exists():
                    console.print("[yellow]ℹ bakFiles/original is missing — no tracked files.\n"
                                  "  Run with -s for an initial scan.[/yellow]")
                else:
                    console.print("[green]✓ No local changes found.[/green]")
                if is_revert:
                    console.print("[green]Nothing to revert — the tree is clean.[/green]")
                    return
                if args.diff:
                    console.print("[yellow]Snapshot is empty — exiting without sync.[/yellow]")
                    return
                sys.exit(run_repo_sync(args.jobs, force=args.forcesync))
        elif is_revert:
            console.print("[green]✓ No local changes found — nothing to revert.[/green]")
            return
        else:
            if not args.smart and not ORIG_DIR.exists():
                console.print("[yellow]ℹ bakFiles/original is missing — no tracked files.\n"
                              "  Run with -s for an initial scan.[/yellow]")
            else:
                console.print("[green]✓ No local changes found.[/green]")
            if args.diff:
                console.print("[yellow]Snapshot is empty — exiting without sync.[/yellow]")
                return
            sys.exit(run_repo_sync(args.jobs, force=args.forcesync))

    display_scan_binding(scan)

    # Final confirmation for -f / -r: the revert cannot be undone in the tree
    # (only the snapshot remains), patches are not applied back.
    if is_force and interactive:
        if not safe_confirm(
                f"FORCE SYNC will revert {len(modified)} modified + remove {len(untracked)} new "
                f"files (the snapshot is kept, patches will NOT come back). Continue?", False):
            console.print("[yellow]Cancelled.[/yellow]")
            return
    if is_revert and interactive:
        if not safe_confirm(
                f"REVERT will revert {len(modified)} modified + remove {len(untracked)} new "
                f"files WITHOUT syncing (the snapshot is kept, patches will NOT come back). Continue?", False):
            console.print("[yellow]Cancelled.[/yellow]")
            return

    # --- Create the snapshot and prepare ---
    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    snap = SnapshotDirs(ts)
    snap.mkdirs()
    console.print(f"\n[bold yellow]--> Snapshot state:[/] {snap.root}")

    mode = "smart" if args.smart else "original"
    # -f / -r already had a global confirmation above — per-file prompts
    # are disabled so that ALL found changes are guaranteed a snapshot and revert.
    untracked, processed = snapshot_pipeline(
        scan, snap, uid, gid,
        clean_original=args.smart,
        interactive=(interactive and not is_force and not is_revert))

    # --- -r mode: revert only, no sync and no file restore ---
    if is_revert:
        # snapshot_pipeline already reverted modified files to HEAD and moved untracked aside.
        # Additionally restore files deleted from the tree to HEAD.
        reverted_deleted = 0
        for repo_rel, data in scan.items():
            repo_abs = ROOT_DIR / repo_rel if repo_rel else ROOT_DIR
            for rel in data.get("deleted", []):
                target = ROOT_DIR / rel
                if target.exists():
                    continue
                try:
                    rel_in_repo = norm_rel(target.resolve().relative_to(repo_abs.resolve()))
                except ValueError:
                    continue
                if git_checkout_head(repo_abs, rel_in_repo):
                    reverted_deleted += 1
                else:
                    head_data = git_show_head(repo_abs, rel_in_repo)
                    if head_data is not None:
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_bytes(head_data)
                        reverted_deleted += 1
        if reverted_deleted:
            console.print(f"[green]✓ Restored deleted files: {reverted_deleted}[/green]")
        n_mod = sum(1 for s in processed.values() if s == "ok")
        n_untracked = len(untracked)
        write_manifest(snap, mode + "+revert", scan, processed,
                       {"reverted_modified": n_mod,
                        "removed_untracked": n_untracked,
                        "reverted_deleted": reverted_deleted})
        fix_permissions(BAK_ROOT, uid, gid)
        console.print(f"\n[bold green]✨ Revert finished (no sync):[/] {snap.root}\n"
                      f"  Reverted modified: {n_mod}, removed new: {n_untracked}, "
                      f"restored deleted: {reverted_deleted}.\n"
                      f"  Everything is saved in the snapshot; restore via -c.")
        return

    # --- -d mode: snapshot only, restore changes to the tree ---
    if args.diff:
        restored = restore_modified_from_snapshot(snap, uid, gid)
        restored += restore_new_files(snap, uid, gid)
        write_manifest(snap, mode + "+diff", scan, processed,
                       {"restored_to_tree": restored})
        fix_permissions(BAK_ROOT, uid, gid)
        console.print(f"\n[bold green]✨ Snapshot ready:[/] {snap.root}\n"
                      f"  Modified files restored to the tree ({restored}). Sync was not run.")
        return

    # --- repo sync (plain or OFOX flow) ---
    sync_rc = do_tree_sync(args, uid, gid, interactive)
    if sync_rc != 0:
        console.print("\n[bold red]❌ repo sync failed![/bold red]")
        if not interactive or safe_confirm("Restore changes from the snapshot back into the tree?", True):
            restored = restore_modified_from_snapshot(snap, uid, gid)
            restored += restore_new_files(snap, uid, gid)
            console.print(f"[yellow]Restored files: {restored}[/yellow]")
        write_manifest(snap, mode, scan, processed, {"sync_rc": sync_rc})
        sys.exit(1)

    # --- Return new files (force sync does not touch them, but we moved them aside) ---
    restored_new = restore_new_files(snap, uid, gid)
    if restored_new:
        console.print(f"[green]✓ Returned new files: {restored_new}[/green]")

    # --- Apply patches (except --forcesync) ---
    apply_results: Dict[str, List[Tuple[str, str]]] = {}
    if args.forcesync:
        console.print("[bold yellow]! --forcesync: patches are NOT applied. "
                      f"Snapshot saved: {snap.root} (apply via -c)[/bold yellow]")
    else:
        console.print("\n[bold yellow]--> Applying saved patches to the updated tree...[/bold yellow]")
        apply_results = apply_patches_from_dir(snap.patches, snap, uid, gid)
        print_apply_summary(apply_results)

    write_manifest(snap, mode + ("+forcesync" if args.forcesync else ""), scan, processed,
                   {"sync_rc": sync_rc,
                    "apply": {k: [rel for rel, _ in v] for k, v in apply_results.items()}})
    fix_permissions(BAK_ROOT, uid, gid)

    failed = len(apply_results.get("failed", []))
    if failed == 0:
        console.print("\n[bold green]✨ Sync completed successfully![/bold green]\n")
    else:
        console.print(f"\n[bold red]⚠️ Done, but {failed} patch(es) failed to apply — "
                      "resolve the conflicts manually (see output above).[/bold red]\n")
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
