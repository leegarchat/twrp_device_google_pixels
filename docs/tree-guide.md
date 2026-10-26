> [Русская версия](tree-guide_ru.md)

# Guide to the `device/google/pixels` tree

`Guide v3 · 2026-10-26 · each file, one line`

Each file - what it is, why it exists, who reads it (build / runtime / developer). Paths are relative to `device/google/pixels/`.

## Entry point and build - start here

| File | Purpose | Read by |
|---|---|---|
| `build.sh` | Single build entry point (AIO-only): fixed `aio` target, lunch, `mka`, cpio extract, installer pack. AI models must not run it (see `build-system.md`). | developer |
| `vendorsetup.sh` | Lunch hook: `TARGET_DEVICE_ALT`/`FOX_TARGET_DEVICES` (all devices), all `FOX_*`/`OF_*`/`TW_*` flags, `OF_FL_PATH1`, writes `.build_platform.conf` | build system, `build.sh` |
| `include/prebuilt/fox_build_callback.sh` | `--second-call` post-processing of the finished ramdisk: config merge, swap-kit assembly, rc surgery, LGZ packing, manifests for reflash | `build.sh` |
| `sync_tree.py` | Smart `repo sync` preserving local edits + snapshot/patch manager (`-s/-d/-c/-f`) | developer |
| `patches/apply_patches.py` | Applies `patches/files/*.patch` to the source tree (`--check` / `--apply`) | `build.sh` |
| `include/prebuilt/check_keymint.sh` | Manual KeyMint check on the device (HAL md5, service status) | developer |

<details>
<summary>📦 Product definition - the universal target</summary>

| File | Purpose |
|---|---|
| `twrp_pixels.mk` | Product definition (`twrp_pixels`, universal target for all Tensor) |
| `device.mk` | Packages, universal ramdisk overlays (`TARGET_RECOVERY_DEVICE_DIRS` = all devices + families), both KeyMint HALs, no first-stage |
| `BoardConfig.mk` | Architecture, partitions, TWRP/OF flags (`TW_FRAMERATE := 120`, brightness, exclusions), fixed `aio` platform, dummy cmdline (stock kernel kept) |
| `Android.mk` / `Android.bp` / `AndroidProducts.mk` | Build inclusion, Soong modules, product list |
| `board-info.txt` | Canonical list of 25 devices for the build fence |

</details>

<details>
<summary>🗂️ Data: families and devices - config, not code</summary>

| Path | Purpose |
|---|---|
| `families/aio/family.mk` | The only build fragment (block size for the intermediate image; stock kernel kept) |
| `families/<fam>/family.conf` | Shell SoC facts for the swap kit: `FAMILY`, `UFS_ADDR`, `USBCTRL`/`USBBUS` |
| `families/<fam>/family.json` | Same + `keymint` (rust\|cpp\|both), common `props` |
| `families/<fam>/recovery.fstab` | Per-family recovery fstab; ships as `recovery.fstab.<fam>` swap kit (live file is the aio placeholder). Stays ext4-only - TWRP detects erofs itself via blkid |
| `families/<fam>/recovery/` | Family ramdisk overlay (rc stubs) |
| `families/<fam>/twrp.flags` | Family `twrp.flags` (UFS paths etc.) on top of the default |
| `families/<fam>/etc/` | VINTF fragments (KeyMint manifests schema 2.0) |
| `families/common/` | Common files without binaries: `recovery.wipe`, `vendor.prop` |
| `devices/<codename>/device.conf` | `DEVICE=` + `FAMILY=` - binding for `build.sh` |
| `devices/<codename>/pixel.json` | Vendor config: modules, partitions, sysfs paths, props (see `device-config.md`) |
| `devices/<codename>/recovery/` | Per-device ramdisk overlay |
| `devices/<codename>/twrp.flags` | Optional override on top of the family file |

</details>

<details>
<summary>🦀 Sources (`include/`) - Rust does the work</summary>

| Path | Purpose |
|---|---|
| `include/recovery-pixel-boot/` | Rust boot engine: init, modules, OTG, thermal, flashlight (see `recovery-engine.md`) |
| `include/recovery-tensor-daemon/` | Rust daemon: storageproxy + weaver for FBE (see `decrypt.md`) |
| `include/recovery-init-stub/` | Static PID 1 (`stub.c` + `snapshot.c`, see `boot-chain.md`) |
| `include/ramdisk_snapshot/` | Rust ramdisk-snapshot fallback (normally built into the stub) |
| `include/prebuilt/lgz_compress_full_x64` / `lgz_compress_lean_arm64` | Host packer / on-device unpacker for the LGZ cluster (prebuilt utilities) |
| `include/source/otg_host_shim/` + `recovery/root/system/lib64/modules/otg/` | Sources and prebuilt `.ko` OTG shim for all kernels |
| `include/source/susfs_rename_fix/` + `.ko` | susfs rename fix |
| `include/fbe_kdf/` (`fox_fbe_kdf.c`) | KDF playground for FBE research + `recovery/root/system/etc/fox_kdf.conf` (pipelines) |

</details>

<details>
<summary>💿 Ramdisk (`recovery/root/`) - what boots</summary>

| Path | Purpose |
|---|---|
| `init.recovery.pixel_common.rc` | Common init: KeyMint/weaver services, OTG, engine calls |
| `init.recovery.usb.rc` | USB configuration (controller address substituted per family) |
| `system/bin/pixelrunatboot.sh` | Engine shell stages (props, slot, modules, firmware) |
| `system/bin/runatboot.sh` | Empty OFox hook (called by `twrp.cpp`; extension point for addons) |
| `system/bin/reflash_twrp.sh` | Reflash recovery from inside recovery |
| `system/bin/{siw,iw}` | Partition reading without mounting + DM mapping, LP utilities |
| `system/bin/*.zip` | Payloads: FIXBACKUPKSU, EXPANDPARTITIONS |
| `system/etc/{fox_kdf.conf,task_profiles.json}` | KDF pipelines, task profiles |
| `system/etc/vintf/` + `vendor/etc/vintf/` | VINTF matrices and manifests in the image |
| `first_stage-ramdisk-files.txt` | First-stage ramdisk file list |

</details>

<details>
<summary>🧩 Miscellaneous - the rest</summary>

| Path | Purpose |
|---|---|
| `docs/` | This documentation + tester guides + `FOX_FLAGS` |
| `include/vendor-ref/bootctrl/` | Bootctrl HAL 1.2 sources (built from source via Soong namespace) |
| `screenshots/` | GUI screenshots for posts and guides |
| `test_/` | Unpacked images for analysis (gitignore) |
| `test_ai_handoff.md`, `test_static_init.md` | Historical session notes (not guides) |

</details>

Key file dependencies are covered in the documents below. Start with: [`build-system.md`](build-system.md).
