# FOX/OF/TW build flags — what they do and what they lead to

Reference for `device/google/pixels` (OrangeFox R12, Android 14 tree).
All paths below are repo-relative to the Android root unless noted.
`pixels/` = `device/google/pixels`.

> Correction note: Stable **does** stop `adbd` at boot
> (`bootable/recovery/twrp.cpp:246-249`, `ctl.stop adbd`) and re-enables
> it only on `main`-page entry (`main.xml:48-65`). Earlier claims that
> Stable leaves `adbd` alone were wrong.

---

## 1. Build branches: Stable vs everything else

There is exactly **one** behavioral Stable-vs-other switch in the `*.mk`
layer (`bootable/recovery/orangefox.mk:151-153`, exact case-sensitive
match on `Stable`). Everything else keyed off the build type is
display-only (strings, props, zip name).

| `FOX_BUILD_TYPE=` | Concrete effects |
|---|---|
| `Stable` (our tree default: `pixels/build.sh --build-type`, `Stable` when omitted; `pixels/vendorsetup.sh` keeps a pre-exported value or defaults) | `OF_ADVANCED_SECURITY:=1` → `-DOF_ADVANCED_SECURITY="1"` (`orangefox.mk:152,224-225`). Runtime: §2. Zip name `...-Stable-...` (`vendor/recovery/OrangeFox_A14.sh:330/332`). `ro.orangefox.type=Stable` (`twrp.cpp:459`), About/`fox_build_type1` strings (`data.cpp:736,898`), `releaseinfo.json` type (`twrp-functions.cpp:367`), Welcome "Build type: Stable + support link" (`twrp-functions.cpp:2735-2738`). |
| `Beta` / `Testing` / `Unstable` / any other non-empty | **No** `OF_ADVANCED_SECURITY` → normal MTP autostart path (`twrp.cpp:292-313`), no early `ctl.stop adbd`. `BETA` (any case) still gets the Welcome support-link line (`twrp-functions.cpp:2736`); other values print "No official support for unknown builds" (`:2740`). Nothing else changes at runtime. |
| unset / empty | Defaults to `"Unofficial"` (`orangefox.mk:143`, `OrangeFox_A14.sh:311`). Same runtime as Beta-row plus Unofficial warning (`twrp-functions.cpp:2732-2733`). |

`FOX_VARIANT` (`pixels/vendorsetup.sh:150` → `default`; `orangefox.mk:43-47`)
and `OF_MAINTAINER` (`:151` → `LeeGarChat`; `orangefox.mk:206-210`,
unset = `"Testing build (unofficial)"`) are **strings only**
(`ro.orangefox.variant`, About page, zip name, `releaseinfo.json`).
No behavior fork. `FOX_BUILD` (`R12.0[_N]`, `orangefox.mk:24-37`,
`--patch N` via `pixels/build.sh:196`) same — version string only.

---

## 2. `OF_ADVANCED_SECURITY=1` — full runtime gate list

Set automatically on `Stable`; manual `export OF_ADVANCED_SECURITY=1`
takes the identical path. Old name `FOX_ADVANCED_SECURITY` is a hard
build error (`orangefox.mk:616-617`).

| Gate | Effect in practice |
|---|---|
| `twrp.cpp:246-249` | **`ctl.stop adbd` at early boot, `orangefox.adb.status=0`. ADB is dead on arrival — including the decrypt password screen.** |
| `twrp.cpp:287-290` | `fox_advanced_security=1`, **`tw_mtp_enabled=0`**, normal MTP autostart skipped (`:292-314` else-branch dead). Log: `ADB & MTP disabled by maintainer`. |
| `main.xml:48-65` | Deferred re-enable **on first `main`-page entry**: `startmtp` + `adb enable` fire once (conditions: no lockscreen pass, `adb_startup=1`, `adb_started!=1`, `tw_has_mtp=1`). **On an encrypted device stuck at the password prompt, `main` is never entered → adb/MTP stay off for the whole session.** |
| `partitionmanager.cpp:812-819` (pixels tree) | Intent comment only: no MTP restart after metadata decrypt (also covered by `tw_mtp_enabled=0`, but explicit). |

**Explicitly NOT gated** (verified by grep — works the same on Stable):
- ADB sideload / ORS (`openrecoveryscript.cpp:408-448` does `ctl.start adbd`; `twrp.cpp:280-282` ORS auto-run unconditional).
- Built-in terminal / nano (`data.cpp:1513-1518`, only `TW_EXCLUDE_NANO` matters).
- File manager (no `fox_advanced_security` conditions).
- `pass.xml:92-106` password-check page (checks only `adb_startup`/`tw_mtp_enabled`).

Practical consequence for testers: on Stable, "no adb on the password
screen" is **by design**, not a bug. Our `otg.rs` `switch_to_device`
self-heal (`ctl.start adbd` + explicit UDC rebind) intentionally revives
it for debugging. Replug cannot help there: init triggers are
edge-based and no new edge ever comes while the screen is up.

---

## 3. Flag inventory (grouped)

Legend: **tree** = set by `pixels/` (vendorsetup.sh unless noted).
"Default" = behavior when unset.

### 3.1 A/B + recovery layout (tree: all set)

| Flag | Values / read-at | Effect | Tree |
|---|---|---|---|
| `FOX_AB_DEVICE` | `1`; `orangefox.mk:93,156-176` | A/B mode: bootctl links, OTA/update-engine paths | `=1` (:158; also forced by `BoardConfig.mk:35` `AB_OTA_UPDATER`) |
| `FOX_VIRTUAL_AB_DEVICE` | `1`; `orangefox.mk:87-95` | Virtual-A/B: implies `FOX_AB_DEVICE=1` + `FOX_VANILLA_BUILD=1`; gates KernelSU, VAB ORS wipe-format | `=1` (:157) |
| `FOX_VENDOR_BOOT_RECOVERY` | `1` (experimental, warns); `orangefox.mk:179-195` | Vendor-boot recovery: forces AB + `OF_NO_SPLASH_CHANGE` + vanilla; repacker/ramdisk layout for vendor_boot | `=1` (:159; matches `BoardConfig.mk:236-237`) |
| `FOX_RECOVERY_VENDOR_BOOT_PARTITION` | block path; `OrangeFox_A14.sh:664-666` (shell post-processing only) | Rewrites `VENDOR_BOOT_PARTITION=` to the real `vendor_boot` node | per-family (:161-167): gs201/gs101→`14700000.ufs`, malibu→`3c2d0000.ufs`, laguna→`3c400000.ufs`, else `13200000.ufs` |
| `FOX_TARGET_DEVICES` / `TARGET_DEVICE_ALT` | comma lists; `orangefox.mk:310-317` | Assert/allow-list (`ro.twrp.target.devices`, OTA checks) | both `="$_ALL_DEVS"` (:175-176, discovered, not hardcoded) |

### 3.2 Vanilla / MIUI (tree: vanilla on)

| Flag | Effect | Tree |
|---|---|---|
| `FOX_VANILLA_BUILD=1` (`orangefox.mk:94,103-114`) | Skips all MIUI/OrangeFox patching (sets a cascade of `OF_SKIP_*`/`OF_DISABLE_*`/`OF_DONT_*`) | `=1` (:171) |
| `OF_DISABLE_MIUI_SPECIFIC_FEATURES=1` (`:106,117-125`) | Strips MIUI menus/patching | `=1` (:172) |

### 3.3 Identity / version (strings only)

`FOX_BUILD_TYPE` (§1, via `build.sh --build-type TYPE`, default `Stable`),
`FOX_VARIANT`, `OF_MAINTAINER` (§1),
`FOX_MAINTAINER_PATCH_VERSION` (whole numbers only or build error;
`build.sh --patch N`; appends `_N` to `R12.0`).

### 3.4 UI geometry (tree: all set)

| Flag | Default | Tree |
|---|---|---|
| `OF_SCREEN_H` | `1920` | `=2400` (:181; runtime override `DOF_SCREEN_H` still wins) |
| `OF_STATUS_H` | `72` | zumapro→`150`, else `130` (:183-190) |
| `OF_STATUS_INDENT_LEFT/RIGHT` | `20` | `=80/80` (:191-192) |
| `OF_HIDE_NOTCH` | `0` | `=1` (:193) |
| `OF_CLOCK_POS` | `0` | `=1` (:194) |
| `OF_ALLOW_DISABLE_NAVBAR` | `1` | `=0` (:195) |
| `OF_OPTIONS_LIST_NUM` | — | `=6` (:196) |

### 3.5 Compression / ramdisk

| Flag | Effect | Tree |
|---|---|---|
| `OF_USE_LZ4_COMPRESSION=1` | LZ4 ramdisk + code path (`BOARD_RAMDISK_USE_LZ4`) | `=1` (:199; `BoardConfig.mk:110` agrees) |
| `FOX_USE_LZ4_COMPRESSION`, `FOX_USE_LZMA_COMPRESSION`, `FOX_ADVANCED_SECURITY`, `OF_PATCH_VBMETA_FLAG`, `OF_TARGET_DEVICES` | **obsolete — hard build errors** (`orangefox.mk:600-621`) | correctly absent |

### 3.6 Feature toggles (tree)

| Flag | Effect | Tree |
|---|---|---|
| `OF_NO_TREBLE_COMPATIBILITY_CHECK=1` | Skips Treble check | `=1` (:203) |
| `OF_ENABLE_LPTOOLS=1` | Includes `lptools` (`TW_INCLUDE_LPTOOLS`, needs `external/lptools`) | `=1` (:204; `BoardConfig.mk:225` duplicates) |
| `OF_USE_GREEN_LED=0` | Green LED off (`-DOF_NO_GREEN_LED`) | `=0` (:205) |
| `OF_NO_SPLASH_CHANGE=1` | Hides splash-change menu (also auto-forced by vendor-boot recovery) | `=1` (:206) |
| `OF_RECOVERY_AB_FULL_REFLASH_RAMDISK=1` | Full ramdisk reflash on A/B (`twrpRepacker.cpp:191,357`) | `=1` (:207) |
| `OF_USE_DMCTL=1` | dmctl over dmsetup | `=1` (:213) |
| `FOX_USE_BASH_SHELL=1` | Ships bash in recovery (`OrangeFox_A14.sh:156,368,422`) | `=1` (:227) |
| `FOX_ENABLE_APP_MANAGER=1` / `FOX_DELETE_AROMAFM=1` | App manager on / AROMA-FM deleted (`:746`) | `=1` (:239-240) |
| `FOX_ENABLE_KERNELSU_SUPPORT=1`, `..._NEXT_SUPPORT=1` | KernelSU/Next support (needs vAB) | `=1` (:258-259) |
| `FOX_DELETE_INITD_ADDON=1` | Deletes init.d addon (default-on unless `=0`) | `=1` (:262) |
| `OF_QUICK_BACKUP_LIST` | Quick-backup preset (`tw_backup_list_quick`) | `="/boot;/vendor_boot;/data;"` (:243) |
| `OF_UNBIND_SDCARD_F2FS=1` | Bind-unmount `/sdcard` before F2FS repair/format | `=1` (:244) |
| `OF_BIND_MOUNT_SDCARD_ON_FORMAT=1` | — | `=1` (:245) |
| `OF_DYNAMIC_FULL_SIZE=8531214336` | Full-size constant (matches `BOARD_SUPER_PARTITION_SIZE`) | (:246) |
| `OF_USE_LEGACY_BATTERY_SERVICES=1` | Legacy battery HAL (`TW_USE_LEGACY_BATTERY_SERVICES`) | `=1` (:249) |
| `OF_FORCE_DATA_FORMAT_F2FS=1` | Force F2FS on format | `BoardConfig.mk:215` (mk, not env) |
| `OF_FL_PATH1="cmd:/system/bin/recovery-pixel-boot torch"` | Torch hook wired to our Rust daemon | (:216) |
| `OF_WORKAROUND_BACKUP_BUG=1` | Forced `1` in mk | default (untouched) |
| `OF_DONT_KEEP_LOG_HISTORY=0`, `FOX_INSTALLER_DISABLE_AUTOREBOOT=0`, `FOX_USE_DATA_RECOVERY_FOR_SETTINGS=0` | Explicit off (defaults would differ) | (:252-254) |

### 3.7 MTP / USB / crypto (tree, mk side)

`TW_THEME:=portrait_hdpi` (`BoardConfig.mk:192`),
`TW_EXCLUDE_APEX/DEFAULT_USB_INIT/TWRPAPP:=true` (`:205-207`),
`TW_USE_TOOLBOX`, `TW_INCLUDE_CRYPTO/CRYPTO_FBE/FBE_METADATA_DECRYPT`,
`TW_INCLUDE_FASTBOOTD/RESETPROP/REPACKTOOLS/NTFS_3G/FUSE_* /LPTOOLS`,
`TWRP_INCLUDE_LOGCAT`, `TW_USE_FSCRYPT_POLICY:=2` (`BoardConfig.mk:196-225`).
`OF_CHECK_OVERWRITE_ATTEMPTS` untouched → check stays enabled.
`OF_SKIP_FBE_DECRYPTION` commented out / `OF_DEFAULT_KEYMASTER_VERSION`
explicitly unset (`vendorsetup.sh:220`).

### 3.8 Toolchain / misc (tree)

`USE_CCACHE=1` (:152), `TARGET_ARCH=arm64` (:153, `BoardConfig.mk:65`
overrides env anyway), `FOX_REPLACE_TOOLBOX_GETPROP=1` (:226),
`FOX_BASH_TO_SYSTEM_BIN=1` (:228), `FOX_LOCAL_CALLBACK_SCRIPT` (:265),
`FOX_KERNEL_VER` / `FOX_KEYMINT_TYPE` / `DEVICE_BUILD_FLAG` / `LGZ_LEVEL`
(via `build.sh`), `VENDOR_BOOT_PATCH_STOCK` (gs101 path).
Commented-out (deliberately off): `FOX_ASH_IS_BASH`, `FOX_USE_TAR_BINARY`,
`FOX_USE_SED_BINARY`, `FOX_USE_XZ_UTILS`, `FOX_USE_UPDATED_MAGISKBOOT`,
`FOX_USE_LZ4_BINARY` (`:229-235`).

### 3.9 Set but never read (dead knobs in this tree)

`OF_IGNORE_LOGICAL_MOUNT_ERRORS=1` (`:202`) — no reader in
`bootable/`, `vendor/recovery/`, `vendor/twrp/`. Legacy/no-op here.
(Unset-but-unused elsewhere: `FOX_REPLACE_BOOTIMAGE_DATE`,
`OF_KEEP_DM_VERITY*` (forced `1` in mk anyway), `OF_FORCE_DISABLE_DM_VERITY`,
`OF_FIX_OTA_UPDATE_DENSITY_ERROR`, `OF_NO_MIUI_OTA_WARNING`,
`FOX_BUGGED_AOSP_ARB_WORKAROUND`, `FOX_RECOVERY_SYSTEM/VENDOR_PARTITION`,
`FOX_SETTINGS_ROOT_DIRECTORY*`, `OF_SKIP_DECRYPTED_ADOPTED_STORAGE`.)

---

## 4. Environment vs mk files

- **Via environment** (exported before/during lunch; make + `OrangeFox_A14.sh`
  post-processing both read them): everything `vendorsetup.sh` exports
  (`FOX_BUILD_TYPE/VARIANT`, `OF_MAINTAINER`, AB/VAB/vendor-boot/vanilla
  families, `FOX_TARGET_DEVICES`, `TARGET_DEVICE_ALT`, all `OF_SCREEN/STATUS/*`,
  `OF_USE_LZ4`, `OF_ENABLE_LPTOOLS`, `OF_USE_*`, `FOX_USE_BASH_SHELL`,
  `FOX_ENABLE_APP_MANAGER`, `FOX_DELETE_*`, `OF_QUICK_BACKUP_LIST`,
  `OF_UNBIND_SDCARD_F2FS`, `FOX_ENABLE_KERNELSU*`, `FOX_RECOVERY_VENDOR_BOOT_PARTITION`,
  `USE_CCACHE`, `TARGET_ARCH`, `DEVICE_BUILD_FLAG`) plus `build.sh` exports
  (`FOX_MAINTAINER_PATCH_VERSION`, `FOX_KERNEL_VER`, `FOX_KEYMINT_TYPE`,
  `LGZ_LEVEL`, `VENDOR_BOOT_PATCH_STOCK`).
- **Must be in mk files** (Soong/mkbootimg/product inheritance read them
  there; env gets overridden): `TARGET_ARCH` (`BoardConfig.mk:65` `:=`),
  all `BOARD_*` (header version, LZ4, partition sizes, metadata, vendor-boot
  moves), `TARGET_RECOVERY_*`, `AB_OTA_*`, `PRODUCT_PLATFORM`, all `TW_*`,
  `OF_FORCE_DATA_FORMAT_F2FS`.

---

## 5. Practical cheat sheet

- Want normal (non-Secure) behavior for a debug build: `build.sh --build-type Beta`
  (or anything but `Stable`) → adbd stays up at boot, MTP autostarts, no
  other runtime change. Zip will say `-Beta-`, Welcome shows support link.
- Stable + adb on password screen: only via `main`-page entry, manual
  Advanced → ADB toggle, ORS/sideload (`ctl.start adbd`), or our otg
  self-heal. Replug does not help (edge-triggered init).
- `Beta` enables **nothing** at runtime vs `Stable` except *removing* the
  §2 gates. There is no "beta features" flag.
- Never set the §3.5 obsolete names — instant build error by design.
