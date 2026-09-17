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
| `siw`, `iw`, `lptools_new`, `lpdump` | Partition reads without mounting, LP tools |
| `nboot.lz4` | Compressed boot component (in LGZ exclusions) |
| `Magisk-*.zip`, `DFENEO.zip`, `FIXBACKUPKSU.zip`, `EXPANDPARTITIONS.zip`, `LeeGar_dfe_neo_healing_*.zip` | Payloads for installation from the GUI |

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

Reflashing recovery without a PC: takes the `kernel_bootcfg` entry
(`FOX_KERNEL_VER` is stamped by the callback from `.gen_kernel.mk`) and file lists
from the callback snapshot manifest. The image is built **without DTB**
(`DTB_SZ 0`, vendor_boot dtb-free) — stripping is mandatory, otherwise
overwriting will clobber adjacent fragments. The `unpack -h`
(header/bootconfig) stamp is written single-record.
