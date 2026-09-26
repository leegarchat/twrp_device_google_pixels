> [Русская версия](ramdisk_ru.md)

# Ramdisk contents (`recovery/root/`)

Only overlays of **its own family** go into the image (`device.mk` filters
`devices/*` by `device.conf`, config merge is by the `family` field). No foreign
`init.recovery.*.rc` files or sections end up in the image.

## Init files (ramdisk root)

| File | Role |
|---|---|
| `init.recovery.pixel_common.rc` | Common init: keymint/weaver services, OTG (`otg_enable`/`otg_auto`), engine `exec` calls on `early-init`/`on init`/`on boot`. Callback surgery target: USB address substitution, disabling the foreign keymint |
| `init.recovery.usb.rc` | USB configuration (DWC3 platform; `11210000` for gs201/zuma/zumapro, replacement via `USBCTRL` for laguna/malibu) |
| `first_stage-ramdisk-files.txt` | First-stage ramdisk file list |

## `system/bin/` — executables

| File | Role |
|---|---|
| `recovery-pixel-boot` | Rust engine (see `recovery-engine.en.md`) |
| `recovery-tensor-daemon` | Weaver/storageproxy daemon (see `decrypt.en.md`) |
| `recovery_init_stub` → `init` | Init stub (swap in the callback, see `boot-chain.en.md`) |
| `pixelrunatboot.sh` | Engine shell stages |
| `runatboot.sh` | Empty OFox hook |
| `reflash_twrp.sh` | Reflash from inside recovery (below) |
| `siw`, `iw` | Partition reads without mounting + DM mapping, LP tools |
| `FIXBACKUPKSU.zip`, `EXPANDPARTITIONS.zip` | Payloads for installation from the GUI |

## `system/etc/` and `vendor/etc/`

| Path | Role |
|---|---|
| `system/etc/fox_kdf.conf` | KDF pipelines (see `decrypt.en.md`) |
| `system/etc/task_profiles.json` | Task profiles |
| `system/etc/vintf/` + `vendor/etc/vintf/` | Compatibility VINTF matrices and HAL manifests (keymint schema 2.0) |
| `vendor/etc/ueventd.rc` | Vendor ueventd rules |

## `system/lib64/modules/`

- `otg/` — `otg_host_shim*.ko` for each kernel branch × pagesize ×
  Android generation (selection by the engine's `ko_picker`).
- `susfs_rename_fix.ko` — susfs rename fix.

## Reflash from inside (`reflash_twrp.sh`)

Reflashing recovery without a PC: snapshots both `vendor_boot` slots,
builds a recovery-only cpio payload from `/ramdisk_snapshot` (first_stage
paths excluded — each slot provides its own), and rebuilds each slot image
from its own stock image via `bootsmasher-install` (smart replace: header,
cmdline and dtb come from the slot itself, nothing is stamped). Both slots
are flashed only if the free-space policy holds, then verified by
fetch-back compare. Stock-image backups go to
`/sdcard/backup_vendor_boot/` when userdata is writable.
