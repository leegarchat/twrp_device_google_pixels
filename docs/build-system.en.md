> [Русская версия](build-system_ru.md)

# Build system (`build.sh`)

## Rules for an AI model exploring this project

If you are an AI model working with this tree, follow these rules:

- **Do not run `build.sh`.** Building is a human's prerogative: it is slow,
  wipes `out/` between groups, hits the `/tmp` limit (16 GB tmpfs), and
  requires decisions (kernel profile, slots, tags). Your run may destroy
  someone else's artifacts or hang on an interactive prompt.
- **Your area:** preparing the tree, writing code/patches/docs, and verifying
  results without building — `apply_patches.py --check`, `patch --dry-run`,
  `bash -n`, `gen_kernel_mk.py --fingerprint`, byte-for-byte comparisons.
- Do not clean `/tmp/pixels/` and `out/` while someone else's build is running.
- `test*` is in `.gitignore` (do not commit test scripts); commit `docs/`.

## Flags

```bash
./build.sh -f shiba -k 6.12 -n test_3 --build-type Beta --force
```

| Flag | Meaning |
|---|---|
| `-f, --family TARGET` | Codename (`shiba`) or family (`zuma`). Without a flag — interactive menu from `vendorsetup.sh` |
| `-k, --kernel VER` | Kernel profile (`6.1`, `6.12`) from `family.json`. Without a flag — interactive selection |
| `--force` | Non-interactive mode. Without `-k` — aborts with a version list (there is no silent default) |
| `-n TAG` | Tag in the image name |
| `--list` | Show the family/device/kernel tree and exit (builds nothing; `[override]` — device-level `kernels`) |
| `--build-type TYPE` | Build type, default `Stable` (details below) |
| `-N, --no-first-stage` | Skip first-stage (vendor_ramdisk) components: no `fstab.*`, no linker/e2fs tools. The recovery ramdisk is unaffected. Reaches `device.mk` as `FOX_NO_FIRST_STAGE=1` |
| `-c, --cpio-only` | Deliver only the ramdisk `cpio.lz4` (`lz4_legacy`), no `.img/.zip`: gs101 → platform fragment (flash with `fastboot flash vendor_boot:`), other families → recovery fragment (`fastboot flash vendor_boot:recovery`) |
| `-j N` | Parallel build jobs (also `-jN`), default nproc |
| `--platform-recovery` | Recovery-in-platform layout (var2-AIO): first-stage + recovery in one ramdisk |
| `--new-theme` | Build the reworked wide-variant theme (`FOX_REWORK_THEME=1`); default is the stock base theme |
| `--push GROUP` | Push the finished AIO zip to Telegram chat(s) (`admin` = admin DMs); non-fatal, failure only warns |
| `-g, --git-tag` | Tag the build in git (`-n` value + datetime); refuses a dirty tree |
| `-D, --diff-tag` | With `--push`: changelog between the previous tag and the fresh tag (text or `changes_<tag>.txt`) |
| `--diff-from TAG` | With `--push`: forced changelog as `TAG..HEAD` (overrides `-D`) |
| `-T, --text TEXT` | With `--push`: postscript appended to the zip message |

Do not use obsolete `-l` (LGZ level) mentions from the script header —
this is the current flag set.

## What happens (5 stages)

1. **Target resolution.** `families/<X>` exists → family. Otherwise
   `devices/<X>/device.conf` is read (`DEVICE`/`FAMILY`) → `DEVICE_BUILD_FLAG=<family>`.
   Lists are built from directories — there are no hardcoded families in the script.
2. **Kernel profiles.** `gen_kernel_mk.py --fingerprint` computes the effective
   profiles and splits devices into groups with identical hashes. It generates
   `families/<fam>/.gen_kernel.mk` (gitignore): `VENDOR_CMDLINE`,
   `BOARD_BOOTCONFIG`, `FOX_KERNEL_VER`. Generation happens **before lunch**, because
   `dumpvars` parses BoardConfig during lunch. Empty
   `VENDOR_CMDLINE` = loud `$(error)`.
3. **Env file.** `vendorsetup.sh` (lunch) writes `.build_platform.conf`
   (family, UFS address, keymint type, LGZ policy): env does not survive
   ninja recipe-shells, so parameters travel via file. Before each group
   the `/tmp/pixels/fox_env.sh` snapshot is refreshed — the post-image hook
   is invoked by the build with a scrubbed environment, and without the file
   `FOX_BUILD_TYPE`/`OUT` were lost (`*-Unofficial-*.img` images in the root).
4. **Soong.** `recovery_init_stub`, `recovery-tensor-daemon`,
   `recovery-pixel-boot`, fstabs, KeyMint HAL are built **from source** (type —
   the `keymint` field in `family.json` → `FOX_KEYMINT_TYPE`). Overlays:
   `TARGET_RECOVERY_DEVICE_DIRS` = root + `devices/*` + own family.
   No foreign rc or sections end up in the image.
5. **Callback (`--second-call`).** `fox_build_callback.sh` runs on the finished
   ramdisk (`$TARGET_DIR`): per-family keymint/VINTF injection, family
   `twrp.flags`, `pixelrunatboot.json` merge (`[PIXELCFG]`), rc surgery
   (USB address, disabling foreign keymint), `post_remove_ramdisk`,
   LGZ packing (`[LGZ]`), snapshot manifest and reflash lists.
   Mandatory `PRODUCT_OUT` cleanup between groups.

## Image grouping

Devices with identical effective profiles (per-flag comparison,
order does not matter) share **one** image (`zuma.img`); a diverged one gets
its own (`zuma_husky.img`, subgroup — `zuma_husky-akita.img`).
Per-device overrides are currently disabled (`_kernels_disabled` in
`pixel.json`) — each family builds into a single shared image; to restore —
rename the key back to `kernels`.

## AIO builds (the default route)

`-f aio` builds **one installer for every supported device** instead of
per-family images:

```bash
./build.sh -f aio --platform-recovery -n test8 -c --build-type Beta
```

What changes in AIO mode: the stock kernel is kept (no kernel image is
built, `-k/--kernel` is meaningless and rejected), the exact device is
detected at runtime and family files (`recovery.fstab`, `twrp.flags`,
USB controller, keymint) are swapped in by the init stub + boot engine.
Output is `builds/OrangeFox-R12.0-test8-aio.zip` — a self-contained
installer (both slots, userdata backup) plus the `*.ramdisk.lz4` payload.
Testers only ever see AIO builds; `-f <family> -k <ver>` remains as a
developer fallback. Push/test flags (`--push`, `-g`, `-D`, `-T`) work the
same on both routes.

## Build types: Stable vs Beta

| | `Stable` (default, exact match) | Any other (`Beta`, …) |
|---|---|---|
| `OF_ADVANCED_SECURITY` | `1` | `0` |
| `adbd` in recovery | stopped (`twrp.cpp`), adb dead by design | alive, root |
| MTP | autostart off | autostart on |

Consequence: on Stable the decryption password is entered only from the screen;
adb-for-password works only on non-Stable builds. For testers —
`--build-type Beta`.
