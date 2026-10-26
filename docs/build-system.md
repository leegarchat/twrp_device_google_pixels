> [Русская версия](build-system_ru.md)

# Build system (`build.sh`)

`Guide v3 · 2026-10-26 · one universal payload`

Single build entry point (AIO-only): fixed `aio` target, lunch, `mka`, cpio extract, installer pack.

<details>
<summary>🤖 Rules for an AI model exploring this project</summary>

- **Do not run `build.sh`.** Building is a human's prerogative: it is slow, wipes `out/`, hits the `/tmp` limit (16 GB tmpfs), and requires decisions (slots, tags). Your run may destroy someone else's artifacts or hang on an interactive prompt.
- **Your area:** preparing the tree, writing code/patches/docs, and verifying results without building - `apply_patches.py --check`, `patch --dry-run`, `bash -n`, `build.sh --list`, byte-for-byte comparisons.
- Do not clean `/tmp/pixels/` and `out/` while someone else's build is running.
- `test*` is in `.gitignore` (do not commit test scripts); commit `docs/`.

</details>

## Flags - the happy path first

```bash
./build.sh -n test8 --build-type Beta
```

| Flag | Meaning |
|---|---|
| `-n TAG` | Name in the payload/installer name; also lands in the in-recovery version string (`R12.0_<TAG>[_<patch>]` via `FOX_BUILD_NAME`) |
| `-p N` | Whole-number patch appended to the in-recovery version (`R12.0[_<name>]_N`) |
| `--build-type TYPE` | Build type, default `Stable` (details below) |
| `--list` | Show the family/device tree and exit (builds nothing) |
| `-j N` | Parallel build jobs (also `-jN`), default nproc |

<details>
<summary>🧰 The rest of the flags</summary>

| Flag | Meaning |
|---|---|
| `-f aio` | Accepted for backward compatibility only (may be omitted). Any other value is **rejected** - there are no per-family builds |
| `--new-theme` | Build the reworked wide-variant theme (`FOX_REWORK_THEME=1`); default is the stock base theme |
| `--push GROUP` | Push the finished AIO zip to Telegram chat(s) (`admin` = admin DMs); non-fatal, failure only warns |
| `-g` | Tag the build in git (`-n` value + datetime); refuses a dirty tree |
| `-D` | With `--push`: changelog between the previous tag and the fresh tag (text or `changes_<tag>.txt`) |
| `--diff-from TAG` | With `--push`: forced changelog as `TAG..HEAD` (overrides `-D`) |
| `-T TEXT` | With `--push`: postscript appended to the zip message |
| `--no-patches` | Skip the maintainer micro-patches step (for test builds on retargeted snapshots) |

Removed: `-k/--kernel`, `--force` (no kernel profiles - stock kernel is kept) - rejected with an error (`build.sh:270-273`); `-c/--cpio-only`, `--platform-recovery`, `-N/--no-first-stage` (always on now) - accepted as no-ops with a NOTE (`build.sh:274-277`).

</details>

<details>
<summary>🏭 What happens (5 stages)</summary>

1. **Target.** Fixed: `DEVICE_BUILD_FLAG=aio` (a non-`aio` `-f` value is rejected). The device list covers every `devices/*/device.conf`.
2. **Stock kernel.** No kernel image is built and no cmdline is composed: `BoardConfig.mk` carries only a dummy `VENDOR_CMDLINE` for the intermediate `vendor_boot` (it donates the platform fragment).
3. **Env file.** `vendorsetup.sh` (lunch) writes `.build_platform.conf` (aio platform, both KeyMint HALs, LGZ level, layout keys): env does not survive ninja recipe-shells, so parameters travel via file. Before the build the `/tmp/pixels/fox_env.sh` snapshot is refreshed - the post-image hook runs with a scrubbed environment, and without the file `FOX_BUILD_TYPE`/`OUT` were lost.
4. **Soong.** `recovery_init_stub`, `recovery-tensor-daemon`, `recovery-pixel-boot` and **both** KeyMint HALs are built **from source**. Overlays: `TARGET_RECOVERY_DEVICE_DIRS` = root + every `devices/*` + every `families/*` - one universal cpio.
5. **Callback (`--second-call`).** `fox_build_callback.sh` runs on the finished ramdisk: both KeyMint HALs + every family's VINTF fragment, `twrp.flags` placeholder + swap kit (`twrp.flags.<fam>` / `recovery.fstab.<fam>` / `recovery.wipe.<fam>` / `families.txt`), `pixelrunatboot.json` merge (`[PIXELCFG]`), USB blanking to UNKNOWN, `post_remove_ramdisk`, LGZ packing (`[LGZ]`), snapshot manifest and reflash lists.

</details>

<details>
<summary>📦 What the build delivers</summary>

- One universal payload instead of per-family images. The stock kernel is kept, the exact device is detected at runtime and family files (`recovery.fstab`, `twrp.flags`, USB controller, KeyMint) are swapped in by the init stub + boot engine.
- Output: `builds/OrangeFox-R12.0-test8-aio.zip` - a self-contained installer (both slots, userdata backup) plus the `*.ramdisk.lz4` payload (platform fragment, `fastboot flash vendor_boot:`). Push/test flags (`--push`, `-g`, `-D`, `-T`) work on this single route.

</details>

## Build types: Stable vs Beta

| | `Stable` (default, exact match) | Any other (`Beta`, …) |
|---|---|---|
| `OF_ADVANCED_SECURITY` | `1` | `0` |
| `adbd` in recovery | stopped at boot, adb dead by design | alive, root |
| MTP | autostart off | autostart on |

Consequence: on Stable the decryption password is entered **from the screen only**; adb-for-password works only on non-Stable builds. For testers - `--build-type Beta`.
