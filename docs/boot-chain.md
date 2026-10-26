> [Русская версия](boot-chain_ru.md)

# Boot chain and init stub

`Guide v3 · 2026-10-26 · from bootloader to GUI`

## Chain (example: shiba)

1. Bootloader → stock kernel + `init_boot` (stock first-stage) + our `vendor_boot`.
2. First-stage: `IsRecoveryMode()` stock check (`access /system/bin/recovery`) → exec of our `/system/bin/init` - a **static stub**, not the real init.
3. Stub: ramdisk snapshot (for reflash) → unpack of the LGZ cluster → exec of the real `fox.init`. Failure in recovery mode = `reboot bootloader`. (`fox.init`, not `init.real`: no collisions with Magisk/KSU chains.)
4. `early-init exec` → `recovery-pixel-boot init` (device resolution, props, hinge detection).
5. `on init exec` → `setup-temp`; Trusty / KeyMint / weaver services start.
6. `on boot`: USB config → `recovery-pixel-boot boot` (synchronous - init waits, modules before GUI) → `start otg_enable` → `patch_dwc3=1` → `start otg_auto`.
7. `recovery` service → GUI → `twrp.cpp` invokes the empty `runatboot.sh`.

<details>
<summary>🧱 Init stub (`include/recovery-init-stub/`)</summary>

- Static C (`static_executable`, no logs) PID 1 in place of `/system/bin/init`; the real init travels **inside the LGZ cluster** as `fox.init` (`init` is on the exclude list, `fox.init` is not; the swap is done by the callback before packing and manifests).
- **First invocation** (`fox.init` missing): built-in snapshot (`snapshot.c`, a port of the Rust version; the Rust `ramdisk_snapshot` binary is kept as a fallback) → `lgz decompress` → marker files (`/lgz_complite`, `/system/etc/lgz_complite`) → exec `fox.init`. The snapshot stays visible at `/ramdisk_snapshot` (+ `/ramdisk_snapshot_manifest.txt`) until reboot - `reflash_twrp.sh` rebuilds from it on demand.
- **Magisk canary:** the stub carries a 16-byte literal that `magiskinit` hexpatches at flash time (stock second-stage path `/data/magiskinit`). When the stub sees its own canary rewritten, it applies the same 16→16 substitution to the `fox.init` file before exec, so the stock first-stage init chain-loads `magiskinit` exactly like on stock+Magisk. Pristine canary = pristine `fox.init`.
- **Fallback chain, first match wins**: markers → `fox.init` → unpack now → `/init` handoff (Magisk/KSU hook; loop-guarded by readlink - if `/init` is this stub, reboot to bootloader instead).
- Unpacking must happen **before** `selinux_setup`: `fox.init`, sepolicy and props must be in place before `SetupSelinux` / `PropertyInit`.
- Why the stub exists at all: files needed **before** unpacking (the unpacker itself, `recovery`, `*.rc`, manifests) cannot travel inside the cluster. The stub is the minimal executable bridge between first-stage and the cluster. Package: `PRODUCT_PACKAGES += recovery_init_stub`.

</details>

<details>
<summary>🗜️ LGZ cluster (ramdisk compression)</summary>

- Ramdisk files are packed into a single solid UCOMP02 cluster `/lgz_cluster.lgz` (host packer `lgz_compress_full_x64`, on-device unpacker `lgz_compress_lean_arm64`), which the stub unpacks before PropertyInit / SELinux / RC parsing. Zips travel inside transparently (ingest as a whole, restored by `lgz decompress`).
- Policies (`LGZ_POLICY`, default `dirs`): `dirs` - only directories from `LGZ_PACK_DIRS` (scripts, libraries, fonts, flashing binaries; `.ko` and `.zip` stay unpacked, extend by appending a directory - `Entries packed` is visible in the log). `all` - everything except exclusions (maximum compression).
- `LGZ_EXCLUDE_LIST` always wins. Entry formats: `"recovery"` (basename anywhere), `"*.rc"` (by suffix), `"twres/dir/file.ext"` (exact ramdisk path), `"twres/subdir/"` (whole tree).
- Rule: everything needed **before** unpacking (stub, `lgz`, `recovery`, `*.rc`, manifests) - exclude list only. The linker/libc closure entries in `LGZ_EXCLUDE_LIST` are currently commented out, so those libs pack into the cluster. Dead weight is removed via `post_remove_ramdisk` in `family.json` (`fox_build_callback.sh`), not via any `LGZ_DROP_LIST` (no such variable exists).

</details>
