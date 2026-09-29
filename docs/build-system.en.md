> [Русская версия](build-system_ru.md)

# Build system (`build.sh`)

## Rules for an AI model exploring this project

If you are an AI model working with this tree, follow these rules:

- **Do not run `build.sh`.** Building is a human's prerogative: it is slow,
  wipes `out/`, hits the `/tmp` limit (16 GB tmpfs), and requires
  decisions (slots, tags). Your run may destroy someone else's artifacts
  or hang on an interactive prompt.
- **Your area:** preparing the tree, writing code/patches/docs, and verifying
  results without building — `apply_patches.py --check`, `patch --dry-run`,
  `bash -n`, `build.sh --list`, byte-for-byte comparisons.
- Do not clean `/tmp/pixels/` and `out/` while someone else's build is running.
- `test*` is in `.gitignore` (do not commit test scripts); commit `docs/`.

## Flags

```bash
./build.sh -n test_3 --build-type Beta
```

| Flag | Meaning |
|---|---|
| `-f, --family aio` | Accepted for backward compatibility only (may be omitted). Any other value is rejected — there are no per-family builds |
| `-n TAG` | Tag in the payload/installer name |
| `--list` | Show the family/device tree and exit (builds nothing) |
| `--build-type TYPE` | Build type, default `Stable` (details below) |
| `-j N` | Parallel build jobs (also `-jN`), default nproc |
| `--new-theme` | Build the reworked wide-variant theme (`FOX_REWORK_THEME=1`); default is the stock base theme |
| `--push GROUP` | Push the finished AIO zip to Telegram chat(s) (`admin` = admin DMs); non-fatal, failure only warns |
| `-g, --git-tag` | Tag the build in git (`-n` value + datetime); refuses a dirty tree |
| `-D, --diff-tag` | With `--push`: changelog between the previous tag and the fresh tag (text or `changes_<tag>.txt`) |
| `--diff-from TAG` | With `--push`: forced changelog as `TAG..HEAD` (overrides `-D`) |
| `-T, --text TEXT` | With `--push`: postscript appended to the zip message |

Removed: `-k/--kernel` and `--force` (no kernel profiles — stock kernel is
kept); `-c/--cpio-only`, `--platform-recovery`, `-N/--no-first-stage`
(always on: the build delivers only the platform-fragment `cpio.lz4`,
first-stage + recovery merged, stock first_stage preserved by the
installer). Old command lines using them still parse (accepted as no-ops).

Do not use obsolete `-l` (LGZ level) mentions from the script header —
this is the current flag set.

## What happens (5 stages)

1. **Target.** Fixed: `DEVICE_BUILD_FLAG=aio` (a non-`aio` `-f` value is
   rejected). The device list covers every `devices/*/device.conf`.
2. **Stock kernel.** No kernel image is built and no cmdline is composed:
   `BoardConfig.mk` carries only a dummy `VENDOR_CMDLINE` for the
   intermediate `vendor_boot` (it donates the platform fragment).
   See `kernel-profiles.en.md` for why profiles are gone.
3. **Env file.** `vendorsetup.sh` (lunch) writes `.build_platform.conf`
   (aio platform, both KeyMint HALs, LGZ level, layout keys): env does not
   survive ninja recipe-shells, so parameters travel via file. Before the
   build the `/tmp/pixels/fox_env.sh` snapshot is refreshed — the post-image
   hook is invoked by the build with a scrubbed environment, and without
   the file `FOX_BUILD_TYPE`/`OUT` were lost (`*-Unofficial-*.img` images
   in the root).
4. **Soong.** `recovery_init_stub`, `recovery-tensor-daemon`,
   `recovery-pixel-boot` and both KeyMint HALs are built **from source**.
   Overlays: `TARGET_RECOVERY_DEVICE_DIRS` = root + every `devices/*` +
   every `families/*` — one universal cpio.
5. **Callback (`--second-call`).** `fox_build_callback.sh` runs on the finished
   ramdisk (`$TARGET_DIR`): both keymint HALs + every family's VINTF
   fragment, `twrp.flags` placeholder + swap kit
   (`twrp.flags.<fam>`/`recovery.fstab.<fam>`/`recovery.wipe.<fam>`/`families.txt`),
   `pixelrunatboot.json` merge (`[PIXELCFG]`), USB blanking to UNKNOWN,
   `post_remove_ramdisk`, LGZ packing (`[LGZ]`), snapshot manifest and
   reflash lists.

## What the build delivers

One universal payload instead of per-family images:

```bash
./build.sh -n test8 --build-type Beta
```

The stock kernel is kept (no kernel image is built), the exact device is
detected at runtime and family files (`recovery.fstab`, `twrp.flags`,
USB controller, keymint) are swapped in by the init stub + boot engine.
Output is `builds/OrangeFox-R12.0-test8-aio.zip` — a self-contained
installer (both slots, userdata backup) plus the `*.ramdisk.lz4` payload
(platform fragment, `fastboot flash vendor_boot:`). Push/test flags
(`--push`, `-g`, `-D`, `-T`) work on this single route.

## Build types: Stable vs Beta

| | `Stable` (default, exact match) | Any other (`Beta`, …) |
|---|---|---|
| `OF_ADVANCED_SECURITY` | `1` | `0` |
| `adbd` in recovery | stopped (`twrp.cpp`), adb dead by design | alive, root |
| MTP | autostart off | autostart on |

Consequence: on Stable the decryption password is entered only from the screen;
adb-for-password works only on non-Stable builds. For testers —
`--build-type Beta`.
