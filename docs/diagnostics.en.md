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

## Full protocol for testers

[`tester-guide.en.md`](tester-guide.en.md) — installation, outcomes A–D,
pstore (`console-ramoops`/`dmesg-ramoops`), `vendor_boot` dump,
fold/tablet UI checks. Reports without logs cannot be fixed.
