> [Русская версия](diagnostics_ru.md)
# On-device diagnostics

adb in recovery is root by default (on Stable builds adbd is stopped
by design — see `build-system.en.md`; for debugging take Beta).

## Baseline poll (first 30 seconds)

```bash
adb shell 'getprop ro.hardware; uname -r; getconf PAGESIZE'
adb shell 'grep -iE "ko_loader|susfs_fix|otg_patch|pixelrunatboot" /tmp/recovery.log'
adb shell 'ls /dev/ko_stage/*/ | head; cat /proc/modules | grep -cE "touch|goodix"'
adb shell 'dmesg | grep -iE "LGZ|OFOX" | head'
```

## Symptom map

| Symptom | Where to look |
|---|---|
| Won't boot at all | `dmesg` from the PC (fastboot), slot (`bootctl`/`slot-detect`), whether both slots are flashed |
| Hangs before GUI | `/tmp/recovery.log`: `boot` stage (modules), `servicemanager.ready`, keymint status |
| No touch | `lsmod` (touch modules from `touch_modules`?), `/dev/input/`, depth guard in `ko-fetch` |
| Won't decrypt | keymint/weaver service status, `.weaver` slot, `ro.crypto.fs_crypto_blkdev` |
| No USB/OTG | `vbus_paths`, `patch_dwc3`, UDC binding in the `otg-auto` log |
| Slow GUI | `TW_FRAMERATE` in the build log (`-DTW_FRAMERATE=` in cflags) |
| Fold: image/touch on different screens | `DOF_SCREEN_W/H`, DSI connector selection (cover patch) |
| Format Data fails | `FoxFstab`/`FoxTeardown`/`FoxFormat` lines in `/tmp/recovery.log` (see below) |

## Format Data on multi-device /data

On units whose stock `fstab` declares a `/data` device set (`device=zoned|exp|exp_alias`,
malibu/laguna families), recovery never builds the f2fs — it mirrors the stock
factory reset. The live geometry is parsed at Format time from the booted slot's
`/first_stage_ramdisk/system/etc/fstab.<family>` (vendor DFE edits and our
snapshots are bypassed); missing nodes are skipped, and without exp members
(empty set, or a lone zoned companion) the legacy single-device path stays
untouched. An unscannable fstab or a declared exp set with zero surviving
nodes refuses the format loudly (fail closed) instead of guessing.

Stages in `/tmp/recovery.log`:
1. `FoxFstab: parsing ...` + `/data set: N device(s)` — what was found;
2. quiesce: MTP off + `FoxTeardown: unmounted ...` (reversible unmounts);
3. merge check (Unmap runs on the unmounted stack), then dm removal top-down
   (`FoxTeardown: removing dm mapping ...`) — past this, a reboot recreates
   the stack via decrypt, keys are still in `/metadata`;
4. `FoxFormat: erasing /data...` — userdata discard with header-kill
   verification, per-companion discards (zoned is left to vold provisioning),
   then `/metadata` wipe, memtag reset, Titan M nuke (`titan-wipe`), wipe of
   `trusty_userdata`, misc flags. `Can_Be_Mounted=false`: vold formats the
   set on the next Android boot («Android sets up /data on its next boot»).
   Header verify proves the 4K header kill only, never the whole wipe;
   crypto-erasure is complete via the `/metadata` wipe + Titan nuke.

`EBUSY` at teardown = something still holds `/data`: reboot recovery
cleanly and retry; nothing is destroyed before that point. `EINVAL`/`EBUSY`
at `removing dynamic partition: userdata_exp.*` = build without the
teardown (update first, then report).

DFE/plaintext units (no live crypto stack) keep legacy rm -rf-flavored
paths — there is nothing to unbind. Plain `mkfs` on a multi-device set
stays unsupported everywhere: tick the rm -rf checkbox for `/data`
wipe/restore, never plain-format.

Honest label: this Format Data is crypto-erasure (kills keys in
`/metadata` + Titan M, discards the set). It does not guarantee physical
zeroing of every UFS block.

## Full protocol for testers

[`tester-guide.en.md`](tester-guide.en.md) — installation, outcomes A–D,
pstore (`console-ramoops`/`dmesg-ramoops`), `vendor_boot` dump,
fold/tablet UI checks. Reports without logs cannot be fixed.
