> [Русская версия](patches_ru.md)
# Patch system (`patches/`)

Third-party code (TWRP, vold, libfstab, keymint glue…) is modified only
via patches. Each patch is stored as a **triplet** — three copies always kept in sync:

```
patches/files/
├── original/<path from source root>   # pristine file (as in AOSP/OFox)
├── modified/<same path>                # full file with our edits
├── patches/<same path>.patch           # unified diff original→modified
├── new/                                  # entirely new files (if needed)
└── source_snapshot.{json,txt}            # historical migration record (not a registry!)
```

## Rules

1. **Edit `modified/`, regenerate `.patch`** with `diff -U3`
   (`--label a/… b/…`, same format as the neighbors), bring the live tree
   to the snapshot. Never the reverse: `.patch` files are not written by
   hand (except backfill exceptions with a mandatory dry-run).
2. **Check:** `apply_patches.py --check` — everything must be `[OK]`
   (target matching the `modified` snapshot). Applying:
   `apply_patches.py --apply --root <build root>` (called by `build.sh`).
   The applier finds files itself via `rglob` — new triplets need no
   registration anywhere.
3. **Live tree = `modified`.** If the tree is already patched (the normal
   state), originals are recovered by reversing the existing patches, not
   by copying from the tree. After editing, the tree is synced — the next
   run will report `already matches modified snapshot`.
4. **Triplet verification:** apply `.patch` to a copy of `original` →
   byte-for-byte equality with `modified` (`cmp`), plus `patch --dry-run`.

## Tool stack

| File | Role |
|---|---|
| `apply_patches.py` | `--check` / `--apply`, dry-run `patch -p0 --fuzz=3`, snapshot drift detection |
| `patchlib.py` | Apply library, structural patches, bypass mode |
| `sync_from_bakfiles.py` | Rebuilding snapshots from `.bak` |
| `sync_tree.py` (root) | Smart `repo sync` preserving locals (`-s` scanner, `-d` diff only, `-c` interactive, `-f` force) |

## What is already covered (57 triplets)

TWRP: `data.cpp` (letterbox), `action.cpp` (`cmd:`-torch), `gui.cpp`,
`objects.hpp`, `pages.cpp`, `patternpassword.cpp`, themes;
`minuitwrp`: `events.cpp` (vibration), `graphics.cpp`, `graphics_drm.cpp`
(cover panel), `resources.cpp`; `partition*.cpp/hpp`,
`twrp-functions.cpp`, `twrpRepacker.cpp`, `install.cpp`, `spl_check.cpp`.
System: vold (`Decrypt`, `MetadataCrypt`, `Weaver1`, `FsCrypt` + headers),
libfstab (`fstab.cpp/.h`), fastbootd (usb_*), keymaster/keymint `Android.bp`,
`keystore2/globals.rs`, update_engine, `init.cpp`/`util.cpp`, Soong/AIDL bridges,
`BoardConfigSoong.mk`. GUI: `TW_FRAMERATE` passthrough (`libguitwrp_defaults.go`),
`listbox.cpp` (scroll without variable).
