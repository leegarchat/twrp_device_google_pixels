> [Русская версия](diagnostics_ru.md)

# On-device diagnostics

`Guide v3 · 2026-10-26 · what to check when it won't boot`

adb in recovery is root by default. On **Stable** builds adbd is stopped by design (see `build-system.md`) - for debugging take **Beta**.

## Baseline poll - first 30 seconds

```bash
adb shell 'getprop ro.hardware; uname -r; getconf PAGESIZE'
adb shell 'grep -iE "ko_loader|susfs_fix|otg_patch|pixelrunatboot" /tmp/recovery.log'
adb shell 'ls /dev/ko_stage/*/ | head; cat /proc/modules | grep -cE "touch|goodix"'
adb shell 'dmesg | grep -iE "LGZ|OFOX" | head'
```

<details>
<summary>🗺️ Symptom map - where to look</summary>

| Symptom | Where to look |
|---|---|
| Won't boot at all | `dmesg` from the PC (fastboot), slot (`bootctl`/`slot-detect`), whether both slots are flashed |
| Hangs before GUI | `/tmp/recovery.log`: `boot` stage (modules), `servicemanager.ready`, KeyMint status |
| No touch | `lsmod` (touch modules from `touch_modules`?), `/dev/input/`, depth guard in `ko-fetch`. Total miss on a modular tree raises `fox.force_hw_gui=1` (VOL+/VOL-/POWER keys); on a monolithic/partial tree the miss is tolerated - check which: `is_monolith_cached` lines in `/tmp/recovery.log` (no `/proc/modules` → true monolith; empty lsmod → monolith; canary `empty_checker_module` with no `/proc/empty_check_gki` trace → partial monolith, Sultan-style: faked RC=0 and empty lsmod) |
| Won't decrypt | KeyMint/weaver service status, `.weaver` slot, `ro.crypto.fs_crypto_blkdev` |
| No USB/OTG | `vbus_paths`, `patch_dwc3`, UDC binding in the `otg_auto` log. Gate blocked (kernel newer than firmware) = device mode only, adb safe. Drive attached but no mount: run `otg_test.sh` on-device (PC unplugged) - checks T1-T8 (disk, part, daemon, `/dev/block/otg-usb` link, uevent match, volumes, mount, dedup) |
| dm confusion | `/tmp/fox_dm_state` (`<name> <dm-target>`, `# v1 epoch=`): the single runtime truth. `dm-watch` is snapshot-only - it never resurrects a node format just destroyed |
| No battery % | health HAL (`android.hardware.health@2.1-service`) alive? `flog.sh` battery snapshot |
| Slow GUI | `TW_FRAMERATE` in the build log (`-DTW_FRAMERATE=` in cflags) |
| Fold: image/touch on different screens | `DOF_SCREEN_W/H`, DSI connector selection (cover patch) |
| Format Data fails | `FoxFstab` / `FoxTeardown` / `FoxFormat` lines in `/tmp/recovery.log` (below) |

</details>

<details>
<summary>💾 Format Data on multi-device `/data` - the two routes</summary>

On units whose stock `fstab` declares a `/data` device set (`device=zoned|exp|exp_alias`, malibu/laguna families), recovery mirrors the stock factory reset. The live geometry is parsed at Format time from the booted slot's `/first_stage_ramdisk/system/etc/fstab.<family>` (vendor DFE edits and our snapshots are bypassed); missing nodes are skipped, and without exp members (empty set, or a lone zoned companion) the legacy single-device path stays untouched. An unscannable fstab or a declared exp set with zero surviving nodes refuses the format loudly (fail closed) instead of guessing.

Stages in `/tmp/recovery.log`:

1. `FoxFstab: parsing ...` + `/data set: N device(s)` - what was found;
2. quiesce: MTP off + `FoxTeardown: unmounted ...` (reversible unmounts);
3. merge check (Unmap runs on the unmounted stack; the sweep skips all crypto dm nodes now), then the route decision on the live stack;
4. decrypted (live dm stack) = route **(a-dm)**, like the legacy flow: `mkfs` through the live crypto dm (vold-exact args, mapper paths + `@alias`) + mount-verify right there; keys survive, `/data` stays usable in-session, no metadata wipe, no Titan nuke. Locked/DFE (no stack) = route **(b)**: dm removal top-down (`FoxTeardown: removing dm mapping ...`; past this, a reboot recreates the stack via decrypt, keys are still in `/metadata`), userdata discard with header-kill verification, per-companion discards (zoned is left to vold provisioning).
5. **(a-dm) failure falls back to (b)** inside the same call. Tail on (b) only: `/metadata` wipe, memtag reset, Titan nuke (`titan-wipe`), wipe of `trusty_userdata`, misc flags; `Can_Be_Mounted=false` on (b) only. The next-boot vold reformat through the fresh dm stack stays authoritative.
6. Header verify proves the 4K header kill only, never the whole wipe; crypto-erasure on route (b) is complete via the `/metadata` wipe + Titan nuke.

`EBUSY` at teardown = something still holds `/data`: reboot recovery cleanly and retry; nothing is destroyed before that point. `EINVAL`/`EBUSY` at `removing dynamic partition: userdata_exp.*` = build without the teardown (update first, then report).

</details>

<details>
<summary>📝 DFE / plaintext and legacy rules</summary>

- DFE/plaintext units (no live crypto stack) keep legacy rm-rf-flavored paths - there is nothing to unbind. Plain `mkfs` on a multi-device set stays unsupported everywhere: tick the rm-rf checkbox for `/data` wipe/restore, never plain-format.
- Ordering rule (legacy `/data`): the rm-rf flavor wipes the live mount **before** any unmount; the mkfs flavor unmounts first. Unmounting before an rm-rf wipe breaks the remount - the stale crypto mapper still holds the raw device (`EBUSY`), or the raw ciphertext is unmountable on its own (`EINVAL`).
- Checked rm-rf on `/data` means a FULL file wipe (same result as mkfs, different method, internal storage included): it bypasses the media-preserving branch, which would otherwise keep `/data/media` and - via the `OF_WORKAROUND_BACKUP_BUG` skip - `/data/data`. Unchecked keeps the preserving behavior (factory-reset style).
- Honest label: this Format Data is **crypto-erasure** (kills keys in `/metadata` + Titan, discards the set). It does not guarantee physical zeroing of every UFS block.

</details>

## Stuck? Logs first

In recovery run `flog.sh` (or `adb shell flog.sh`). One bundle `fox_logs_<device>_<family>_<stamp>.tar.gz`, attach it + device model. No logs = no fix. Full field protocol (outcomes A-D, pstore, `vendor_boot` dump, fold/tablet UI checks): [`tester-guide.md`](tester-guide.md).
