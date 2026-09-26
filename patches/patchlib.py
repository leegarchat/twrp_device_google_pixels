from __future__ import annotations

import difflib
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence


STATUS_PREFIX = {
    "applied": "OK",
    "patched": "PATCHED",
    "would_patch": "PENDING",
    "would_create": "PENDING",
    "created": "CREATED",
    "bypassed": "BYPASS",
    "failed": "ERROR",
}


@dataclass
class PatchResult:
    id: str
    title: str
    status: str
    message: str
    target: str = ""
    details: str = ""
    manual_hint: str = ""

    def format(self) -> str:
        prefix = STATUS_PREFIX.get(self.status, self.status.upper())
        target = f" [{self.target}]" if self.target else ""
        return f"[{prefix}] {self.id}{target}: {self.message}"


@dataclass
class PatchContext:
    root: Path
    config: dict
    apply: bool
    apply_bypassed: bool
    verbose: bool = False

    def resolve(self, relative_path: str | Path) -> Path:
        return self.root / Path(relative_path)

    def should_bypass(self, patch: "BasePatch") -> bool:
        if self.apply_bypassed:
            return False

        patch_config = self.config.get("patches", {}).get(patch.id, {})
        mode = patch_config.get("mode")
        if mode == "apply":
            return False
        if mode == "bypass":
            return True

        source_path = Path(patch.source_path) if patch.source_path else None
        for bypass_path in self.config.get("bypass_paths", []):
            if source_path and _is_relative_to(source_path, Path(bypass_path)):
                return True

        return patch.bypass_by_default


class BasePatch:
    id: str
    title: str
    source_path: str | None = None
    bypass_by_default = False
    manual_hint = "Apply the matching files/patches/*.patch manually, then rerun the launcher."

    def run(self, context: PatchContext) -> PatchResult:
        raise NotImplementedError

    def result(
        self,
        status: str,
        message: str,
        target: str = "",
        details: str = "",
        manual_hint: str | None = None,
    ) -> PatchResult:
        return PatchResult(
            id=self.id,
            title=self.title,
            status=status,
            message=message,
            target=target,
            details=details,
            manual_hint=manual_hint if manual_hint is not None else self.manual_hint,
        )


@dataclass
class Hunk:
    old_block: list[str]
    new_block: list[str]
    old_range: tuple[int, int]
    new_range: tuple[int, int]


class SnapshotPatch(BasePatch):
    """Structural patch: original/modified are ABSOLUTE snapshot files in
    patches/files/{original,modified}; target is android-tree-relative."""

    def __init__(
        self,
        patch_id: str,
        title: str,
        target: str,
        original_file: Path,
        modified_file: Path,
        patch_file: Path | None = None,
        original_replacements: Sequence[tuple[str, str]] = (),
        context_lines: int = 5,
        bypass_by_default: bool = False,
        manual_hint: str | None = None,
    ) -> None:
        self.id = patch_id
        self.title = title
        self.target = target
        self.original_file = Path(original_file)
        self.modified_file = Path(modified_file)
        self.patch_file = Path(patch_file) if patch_file else None
        # source_path is used only for bypass matching; keep repo-relative modified ref
        self.source_path = f"device/google/pixels/patches/files/modified/{target}"
        self.original_replacements = list(original_replacements)
        self.context_lines = context_lines
        self.bypass_by_default = bypass_by_default
        if manual_hint:
            self.manual_hint = manual_hint
        elif self.patch_file is not None:
            self.manual_hint = (
                f"Port patches/files/modified/{target} into {target} "
                f"(or apply patches/files/patches/{target}.patch with `patch -p1`), "
                f"then rerun the patcher."
            )
        else:
            self.manual_hint = (
                f"Port patches/files/modified/{target} into {target}, then rerun the patcher."
            )

    def run(self, context: PatchContext) -> PatchResult:
        target_path = context.resolve(self.target)

        missing = [str(p) for p in (self.original_file, self.modified_file) if not p.exists()]
        if missing:
            return self.result(
                "failed",
                "required patch input is missing",
                self.target,
                details="\n".join(missing),
                manual_hint=f"Missing file(s): {', '.join(missing)}",
            )
        if not target_path.exists():
            return self.result(
                "failed",
                "target file is missing in tree",
                self.target,
                manual_hint=self.manual_hint,
            )

        original_lines = _read_lines(self.original_file)
        modified_lines = _read_lines(self.modified_file)
        target_lines = _read_lines(target_path)
        original_lines = _replace_in_lines(original_lines, self.original_replacements)

        if target_lines == modified_lines:
            return self.result("applied", "target already matches modified snapshot", self.target)

        hunks = _build_hunks(original_lines, modified_lines, self.context_lines)
        if not hunks:
            return self.result("applied", "no snapshot differences", self.target)

        working_lines = list(target_lines)
        changed_hunks: list[Hunk] = []
        conflicts: list[str] = []

        for index, hunk in enumerate(hunks, start=1):
            if _contains_block(working_lines, hunk.new_block):
                continue

            if context.should_bypass(self):
                return self.result("bypassed", "patch is configured for bypass", self.target)

            matches = _find_block_indexes(working_lines, hunk.old_block)
            if len(matches) == 1:
                start = matches[0]
                working_lines = (
                    working_lines[:start]
                    + hunk.new_block
                    + working_lines[start + len(hunk.old_block) :]
                )
                changed_hunks.append(hunk)
                continue

            if len(matches) > 1:
                conflicts.append(
                    f"hunk {index}: old structure matched {len(matches)} locations "
                    f"around original lines {hunk.old_range[0]}-{hunk.old_range[1]}"
                )
            else:
                conflicts.append(
                    f"hunk {index}: neither modified nor original structure matched "
                    f"around original lines {hunk.old_range[0]}-{hunk.old_range[1]}"
                )

        if conflicts:
            # Fallback: try unified diff via `patch` before giving up.
            # Guarded: a STALE stored patch can reverse-apply cleanly on a
            # tree that holds an older snapshot ("already applied") while
            # the current snapshot hunks are absent — that masks drift and
            # ships stale code silently. Accept the fallback only when every
            # current new_block is verifiably present afterwards.
            fallback = self._try_unified_fallback(context, target_path)
            if fallback is not None:
                if hunks and all(
                    _contains_block(_read_lines(target_path), h.new_block) for h in hunks
                ):
                    return fallback
                conflicts.append(
                    "unified fallback does not cover current snapshot "
                    "(stored .patch is stale; refresh it from modified/)"
                )
            details = "\n".join(conflicts)
            if self.patch_file is not None and self.patch_file.exists():
                details += f"\nManual patch available: {self.patch_file}"
            return self.result(
                "failed",
                "snapshot drift; cannot apply safely",
                self.target,
                details=details,
                manual_hint=self.manual_hint,
            )

        if not changed_hunks:
            return self.result("applied", "all modified structures are already present", self.target)

        if not context.apply:
            return self.result(
                "would_patch",
                f"would apply {len(changed_hunks)} structural hunk(s)",
                self.target,
            )

        target_path.write_text("".join(working_lines), encoding="utf-8")
        return self.result(
            "patched",
            f"applied {len(changed_hunks)} structural hunk(s)",
            self.target,
        )

    def _try_unified_fallback(
        self, context: PatchContext, target_path: Path
    ) -> PatchResult | None:
        if self.patch_file is None or not self.patch_file.exists():
            return None
        patch_bytes = self.patch_file.read_bytes()
        # already applied?
        dry_reverse = subprocess.run(
            ["patch", "-p0", "-R", "--dry-run", "-s", "-l", "--fuzz=3", str(target_path)],
            input=patch_bytes,
            capture_output=True,
        )
        if dry_reverse.returncode == 0:
            return self.result("applied", "unified patch already applied", self.target)
        dry = subprocess.run(
            ["patch", "-p0", "-N", "--dry-run", "-s", "-l", "--fuzz=3", str(target_path)],
            input=patch_bytes,
            capture_output=True,
        )
        if dry.returncode != 0:
            return None
        if not context.apply:
            return self.result("would_patch", "unified patch would apply", self.target)
        res = subprocess.run(
            ["patch", "-p0", "-N", "-s", "-l", "--fuzz=3", "--no-backup-if-mismatch",
             str(target_path)],
            input=patch_bytes,
            capture_output=True,
        )
        for junk in (Path(f"{target_path}.rej"), Path(f"{target_path}.orig")):
            junk.unlink(missing_ok=True)
        if res.returncode == 0:
            return self.result("patched", "applied via unified patch fallback", self.target)
        return None


class NewFilePatch(BasePatch):
    """New (untracked) file: source is ABSOLUTE file in patches/files/new."""

    def __init__(
        self,
        patch_id: str,
        title: str,
        target: str,
        source_file: Path,
        bypass_by_default: bool = False,
        manual_hint: str | None = None,
    ) -> None:
        self.id = patch_id
        self.title = title
        self.target = target
        self.source_file = Path(source_file)
        self.source_path = f"device/google/pixels/patches/files/new/{target}"
        self.bypass_by_default = bypass_by_default
        self.manual_hint = manual_hint or (
            f"Copy patches/files/new/{target} into {target}, then rerun the patcher."
        )

    def run(self, context: PatchContext) -> PatchResult:
        target_path = context.resolve(self.target)
        if not self.source_file.exists():
            return self.result(
                "failed", "new-file source is missing", self.target,
                details=str(self.source_file),
            )
        wanted = self.source_file.read_bytes()
        if target_path.exists():
            if target_path.read_bytes() == wanted:
                return self.result("applied", "new file already present", self.target)
            return self.result(
                "failed", "target exists but differs from new-file snapshot",
                self.target, manual_hint=self.manual_hint,
            )
        if context.should_bypass(self):
            return self.result("bypassed", "new file is configured for bypass", self.target)
        if not context.apply:
            return self.result("would_create", "would create new file", self.target)
        target_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(self.source_file, target_path)
        return self.result("created", "created new file from snapshot", self.target)


def _read_lines(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8").splitlines(keepends=True)


def _replace_in_lines(lines: Iterable[str], replacements: Sequence[tuple[str, str]]) -> list[str]:
    replaced = []
    for line in lines:
        for old, new in replacements:
            line = line.replace(old, new)
        replaced.append(line)
    return replaced


def _build_hunks(original: list[str], modified: list[str], context_lines: int) -> list[Hunk]:
    matcher = difflib.SequenceMatcher(None, original, modified)
    hunks = []
    for group in matcher.get_grouped_opcodes(context_lines):
        old_start = group[0][1]
        old_end = group[-1][2]
        new_start = group[0][3]
        new_end = group[-1][4]
        hunks.append(
            Hunk(
                old_block=original[old_start:old_end],
                new_block=modified[new_start:new_end],
                old_range=(old_start + 1, old_end),
                new_range=(new_start + 1, new_end),
            )
        )
    return hunks


def _find_block_indexes(haystack: Sequence[str], needle: Sequence[str]) -> list[int]:
    if not needle or len(needle) > len(haystack):
        return []
    length = len(needle)
    return [index for index in range(len(haystack) - length + 1) if haystack[index : index + length] == list(needle)]


def _contains_block(haystack: Sequence[str], needle: Sequence[str]) -> bool:
    return bool(_find_block_indexes(haystack, needle))


def _extract_marked_block(text: str, start_marker: str, end_marker: str) -> str:
    start = text.find(start_marker)
    if start < 0:
        raise RuntimeError(f"start marker not found: {start_marker}")
    end = text.find(end_marker, start)
    if end < 0:
        raise RuntimeError(f"end marker not found: {end_marker}")
    line_end = text.find("\n", end)
    if line_end < 0:
        line_end = len(text)
    return text[start:line_end + 1]


def _is_relative_to(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False
