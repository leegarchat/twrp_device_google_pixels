> [Русская версия](ramdisk_ru.md)

# Ramdisk contents (`recovery/root/`)

`Guide v3 · 2026-10-26 · what ships inside the image`

Overlays of **every device + every family** go into the one universal cpio (`TARGET_RECOVERY_DEVICE_DIRS` = root + all `devices/*` + all `families/*` in `device.mk`). The installer + boot engine select the right family files at install/boot time.

<details>
<summary>📜 Init files (ramdisk root)</summary>

| File | Role |
|---|---|
| `init.recovery.pixel_common.rc` | Common init: KeyMint/weaver services, OTG (`otg_enable`/`otg_auto`), engine `exec` calls on `early-init`/`on init`/`on boot`. Callback surgery target: USB address substitution, disabling the foreign KeyMint |
| `init.recovery.usb.rc` | USB configuration (DWC3 platform; `11210000` for gs201/zuma/zumapro, replacement via USBCTRL for laguna/malibu) |
| `first_stage-ramdisk-files.txt` | First-stage ramdisk file list |

</details>

<details>
<summary>⚙️ `system/bin/` - executables</summary>

| File | Role |
|---|---|
| `recovery-pixel-boot` | Rust engine (see `recovery-engine.md`) |
| `recovery-tensor-daemon` | Weaver/storageproxy daemon (see `decrypt.md`) |
| `recovery_init_stub` → `init` | Init stub (swapped in by the callback, see `boot-chain.md`) |
| `pixelrunatboot.sh` | Engine shell stages (props, slot, modules, firmware) |
| `runatboot.sh` | Empty OFox hook (called by `twrp.cpp`; extension point for addons) |
| `reflash_twrp.sh` | Reflash from inside recovery (below; tool: `/system/bin/bootsmasher-install`, staged by the build callback from `include/prebuilt/bootsmasher-install-arm64`) |
| `flog.sh` | One-shot log collector: `fox_logs_<device>_<family>_<stamp>.tar.gz` → `/data/media/0/fox_logs/` (or `/tmp`, + `adb pull` hint). Packs recovery/weaver/dmesg/logcat, backlight + battery/health snapshot, pstore, installer logs |
| `otg_test.sh` | OTG field test T1-T8 (disk, part, daemon, `/dev/block/otg-usb` link, uevent match, volumes, mount, dedup) → `otg_test_<device>_<family>_<stamp>.tar.gz` next to the flog archives. Run on-device with the drive attached and the PC unplugged |
| `siw`, `iw` | Partition reads without mounting + DM mapping, LP utilities |
| `lptools_new` | DM mapping via the system fs_mgr stack (`--map`/`--unmap`), staging fallback |
| `fox-hsinstall` | High-speed ROM install engine (static Rust prebuilt) |
| `FIXBACKUPKSU.zip`, `EXPANDPARTITIONS.zip` | Payloads for installation from the GUI |

</details>

<details>
<summary>📁 `system/etc/`, `vendor/etc/`, modules</summary>

| Path | Role |
|---|---|
| `system/etc/fox_kdf.conf` | KDF pipelines (see `decrypt.md`) |
| `system/etc/task_profiles.json` | Task profiles |
| `system/etc/vintf/` + `vendor/etc/vintf/` | Compatibility VINTF matrices and HAL manifests (KeyMint schema 2.0) |
| `vendor/etc/ueventd.rc` | Vendor ueventd rules |
| `system/lib64/modules/otg/` | `otg_host_shim*.ko` / `aoc_vote_shim*.ko` for each kernel branch × pagesize × Android generation (picked by the engine's `ko_picker`) |
| `system/lib64/modules/probe/` | `empty_checker_module` canary set (monolith / inclusion proving, same kver/android/pagesize layout) |
| `system/lib64/modules/touch/` | Per-device touch archives (`<device>.lgz`, injected by the installer via `--inject-lgz`, unpacked by the `touch-unpack` stage) |
| `susfs_fix` | susfs rename fix |

</details>

<details>
<summary>🔄 Reflash from inside (`reflash_twrp.sh`)</summary>

- Reflashing recovery without a PC (`bootsmasher-install install --file`): snapshots of **both** `vendor_boot` slots are taken, each slot image is rebuilt from its **own** stock image (smart replace: first_stage, header, cmdline and dtb come from the slot itself, nothing is stamped). Missing `/ramdisk_snapshot` is rebuilt on demand from `/ramdisk_snapshot_manifest.txt` (one cpio pass, no helper binary).
- A recovery-only cpio payload is built from `/ramdisk_snapshot` (first_stage paths excluded - each slot provides its own; the live `/<device>.lgz` is re-appended to the manifest so the touch package survives). Both slots are flashed only if the MIN_FREE_MB policy holds, then verified by fetch-back compare.
- Stock-image backups go to `/sdcard/backup_vendor_boot/` when userdata is writable.

</details>
