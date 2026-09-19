# Testing OrangeFox test builds — full guide (EN)

This guide is for testers of **experimental (test) OrangeFox builds**.
Test builds can fail to boot, break decryption, or leave a slot unbootable —
follow this guide exactly and you will always have a way back.

---

## 0. Prerequisites

- PC with `platform-tools` (`fastboot`, `adb`) installed and working.
- USB cable + preferably a USB 2.0 port/hub (fewer flaky connections).
- Phone battery **> 50%**.
- **Factory images** for your exact device (from your firmware) downloaded and unpacked
  (you need at least the factory `vendor_boot.img` for your slot).
- Know your device codename (`shiba`, `comet`, `grizzly`, …) and current slot:
  ```
  fastboot getvar current-slot
  ```
- Back up anything important. Test builds: **never Format Data unless asked**.
- **Verify the image hash BEFORE flashing.** Telegram sometimes delivers
  `.img` files truncated (this really happened during R11 testing and cost
  days of pointless debugging). The maintainer posts an `md5` next to every
  test build (or pinned in https://t.me/OFRPforTensor) — compare:
  ```
  md5sum OrangeFox-test-xxx.img
  ```
  (Windows: `certutil -hashfile OrangeFox-test-xxx.img MD5`.) If it does not
  match — re-download, preferably from the GDrive folder linked in the
  channel. **Bootloop + empty pstore with a mismatched hash = broken
  download, not a bug — do not report it, re-download first.**

---

## 1. Flashing a test build

The maintainer will tell you **which file to flash and into which slot**.
Default procedure (example: slot A):

```
fastboot flash vendor_boot_a OrangeFox-test-xxx.img
fastboot reboot recovery
```

### Pixel 6 series (gs101: oriole/raven/bluejay) — special procedure

Pixel 6 has no `vendor_kernel_boot` partition, so you do **NOT** flash
the `.img` — you flash the `.ramdisk.lz4` **ramdisk** into the platform
fragment (note the trailing colon = empty fragment name):

```
fastboot flash vendor_boot_a: OrangeFox-test-xxx-gs101.ramdisk.lz4
fastboot reboot recovery
```

(`:default` or `:recovery` are WRONG here — the first collapses the
whole table and kills stock dlkm, the second overwrites the wrong
fragment.)

Backup first (bootloader, no root needed) — both slots:
```
fastboot fetch vendor_boot_a ./vendor_boot_a.stock.img
fastboot fetch vendor_boot_b ./vendor_boot_b.stock.img
```
Size must be 67108864 bytes (64 MB). Rollback:
```
fastboot flash vendor_boot_a vendor_boot_a.stock.img   # or _b
```

Rules:

1. Flash **only** the partition you were told to (normally `vendor_boot_X`).
   Never flash `super`, `boot`, `system`, `modem`, bootloader or radio
   from a test package unless explicitly instructed.
2. Know which slot is active **before** flashing. If unsure, flash the
   active slot only, so the other slot stays a known-good fallback.
3. Keep the factory (known-good) `vendor_boot.img` at hand for instant rollback:
   ```
   fastboot flash vendor_boot_a stock_vendor_boot.img   # or _b (factory, known-good)
   fastboot reboot
   ```

---

## 2. First boot — the 60-second checklist

After `fastboot reboot recovery`, wait up to 60 seconds (USB stack and
decryption services start with a delay), then run:

> **adb on the password screen (non-Stable test builds).** You received a
> NON-Stable (Beta) test build — it does not force-disable adb/MTP the way
> Stable builds do. So adb on the initial PIN prompt **may work or may not —
> the developer genuinely does not know**: their own device is currently
> unencrypted, so they have no way to check this path. Your job is to
> record the fact.
> - If `adb devices` sees the device right on the password screen — great,
>   run all commands below right there and report that adb on password
>   **was present**.
> - If `no devices` — also fine for the test: leave the password screen
>   (enter the PIN if touch works, or **Skip** button / back key to the main
>   menu without decrypting — may be absent on some builds; then PIN is the
>   only way out) and run the commands after. Report that adb on password
>   **was absent**.
> The logs (`recovery.log`, `weaver.log`) are written from the very start in
> both cases — pulling them later loses nothing.

```
adb devices -l
adb shell 'getprop sys.usb.config; getprop init.svc.vendor.keymint.rust-trusty; getprop init.svc.vendor.keymint-trusty; uname -r; getprop ro.boot.slot_suffix'
```

What good looks like:

| Check | Healthy value |
|---|---|
| `adb devices` | device listed as `recovery` |
| `sys.usb.config` | `mtp,adb` |
| keymint service | `running` (exactly one of rust-trusty / trusty) |
| `uname -r` | kernel the build was made for (ask maintainer if unsure) |
| slot suffix | the slot you flashed (`_a` / `_b`) |

Then pull the two main logs — as soon as adb is available (right on the
password screen if present, or after the PIN / leaving it — the files
accumulate from boot, they are the most valuable artifact):

```
adb pull /tmp/recovery.log
adb pull /tmp/weaver.log
```

And the full kernel + system logs. For any hardware-level debugging
(touch, display, USB, decrypt, sensors) these are the most important
files after `recovery.log` itself — always full files, never excerpts:

```
adb shell 'dmesg > /tmp/dmesg.log; logcat -d -v time > /tmp/logcat.log'
adb pull /tmp/dmesg.log
adb pull /tmp/logcat.log
```

---

## 3. Decryption test

1. If the device asks for PIN/password/pattern — enter it.
2. Success = you see your files (Internal Storage) and `recovery.log`
   contains `User 0 Decrypted`.
3. If touch does not work: **that is NOT a test failure** unless the
   maintainer said otherwise. Report it, continue over `adb`.
4. Send back: `recovery.log` (full file), `weaver.log`, `dmesg.log`,
   `logcat.log`, and whether `/data/media` is visible.

---

## 4. How the UI renders (folds and tablets especially!)

On devices with the new display logic (folds, tablets) this is a separate
report item — look closely and record:

1. **Which screen shows the UI** (fold: cover/outer or inner? Was the device
   folded or unfolded at boot?).
2. **Black bars**: present? On which sides (left/right, top/bottom, all four)?
   Bars are the normal letterbox mode — their presence/absence and symmetry
   matter.
3. **Stretching**: do circles look like circles or ellipses? Are text and icons
   at the same scale as the frames, or do they "float"? Is the keyboard full
   width or narrower than the screen?
4. **Rotation**: is the UI upright or turned 90°? (The fold inner screen may
   rotate — report which way.)
5. **Touch-to-screen match**: do taps land where shown? (On a fold separately:
   does the outer screen touch work on the outer screen, not vice versa?)
6. **Photograph the screen** with another phone — a photo beats any words.
   For a fold: photos folded + unfolded.

If adb is available, add to the report:
```
adb shell 'getprop DOF_SCREEN_W; getprop DOF_SCREEN_H; getprop DOF_PROGRESSIVE_SCALE'
```

## 4.1. Feature tests (only if asked)

- **Touch**: works / partially / dead + `dmesg | grep -i touch`.
- **Torch / haptics**: works or not.
- **OTG / USB mouse**: works or not.
- **Format Data / Reflash Recovery**: **FORBIDDEN on test builds**
  unless the maintainer explicitly allows it (some test builds refuse
  reflash by design — report the refusal text, it confirms a guard works).

---

## 5. Outcome A — boots, adb works

Do sections 2–4 and send the report (see §8). You are done.

## 6. Outcome B — boots, but NO adb

1. Wait a full 60 s, unplug/replug the cable, try another port / USB 2.0 hub.
2. Check fastbootd: `fastboot devices`. If the device is visible in
   fastbootd, the kernel is alive and it is a USB-gadget problem — report it.
3. If completely silent, go to Outcome C (rollback).

## 7. Outcome C — does NOT boot (logo / black screen / bootloop)

**Do not panic. Do not flash random things.** Roll back:

1. Flash the **factory (known-good)** `vendor_boot.img` into the **same slot** you used:
   ```
   fastboot flash vendor_boot_a stock_vendor_boot.img   # or _b (factory, known-good)
   ```
2. Boot to system normally.
3. With root, capture the kernel crash dumps **before they are overwritten**
   (a second boot cycle may rotate them — be quick, one boot only):
   ```
   su
   ls /sys/fs/pstore/
   cat /sys/fs/pstore/console-ramoops-0 > /sdcard/ramoops_console.txt
   cat /sys/fs/pstore/dmesg-ramoops-0  > /sdcard/ramoops_dmesg.txt
   dmesg > /sdcard/dmesg_boot.txt
   getprop ro.boot.slot_suffix; uname -r
   ```
4. Pull the files to PC (`adb pull /sdcard/ramoops_console.txt` …) and send
   **all of them** plus: which file was flashed, into which slot.

### The boot → crash → switch-slot / boot-to-system trick

- If slot B with the test build bootloops, the bootloader may mark it
  unbootable and fall back to slot A by itself — check
  `fastboot getvar current-slot` after the failure.
- You can also force the other slot: `fastboot --set-active=a` (or `b`),
  then boot to system and collect the pstore dumps as above.
- Key point: **pstore survives a reboot but not many** — always collect
  dumps on the FIRST successful boot after the crash, do not reboot twice.

## 8. Outcome D — boots with adb, but decryption FAILS

Send:

1. Full `/tmp/recovery.log` (not excerpts).
2. Output of a manual HAL run (run ~15 seconds, then Ctrl-C):
   ```
   adb shell '/vendor/bin/hw/android.hardware.security.keymint-service.rust.trusty --dev /dev/trusty-ipc-dev0'
   ```
   (If your build uses the C++ HAL, the maintainer will give another path.)
3. Service states:
   ```
   adb shell 'getprop init.svc.recovery_storageproxyd; getprop init.svc.recovery_weaver'
   ```

---

## 9. Report template (copy-paste)

```
Build file : <exact file name>
Flashed to : vendor_boot_a / vendor_boot_b
Device     : <codename, e.g. grizzly>
Outcome    : boots+adb / boots-no-adb / no-boot / boots-no-decrypt
adb visible: yes / no
adb on password: present / absent (if device encrypted)
decrypt    : ok / failed / not tried
touch      : ok / dead / partial
UI         : bars (which sides) / stretched / rotated / which screen (fold: folded/unfolded) + photo
Attached   : recovery.log, weaver.log, dmesg.log, logcat.log, (ramoops_console.txt, ramoops_dmesg.txt, dmesg_boot.txt if Outcome C)
Notes      : <anything unusual: how long boot took, error texts, ...>
```

---

## 10. Golden rules

1. One change at a time. Never combine a test build with other mods.
2. Factory (known-good) images ready **before** flashing, not after.
3. Logs first, conclusions later — always attach full files.
4. Do NOT press “Reflash Recovery”, do NOT Format Data, do NOT flash
   other partitions on test builds unless told to.
5. If something behaves unexpectedly — photograph/write down the exact
   text. “It didn't work” is not a report.

## 11. How to dump your factory vendor_boot (backup before testing)

No factory images at hand? Dump the known-good `vendor_boot` straight from
the device **before** flashing the test build. Easiest — bootloader, no
root needed (works for Pixel 6 too):
```
fastboot fetch vendor_boot_a ./vendor_boot_a.stock.img
fastboot fetch vendor_boot_b ./vendor_boot_b.stock.img
```

Alternatively, needs root (in system) or recovery:

From rooted system (both slots at once):
```
su
dd if=/dev/block/by-name/vendor_boot_a of=/sdcard/vendor_boot_a.stock.img bs=4M
dd if=/dev/block/by-name/vendor_boot_b of=/sdcard/vendor_boot_b.stock.img bs=4M
ls -l /sdcard/vendor_boot_*.stock.img
```

From recovery (adb, root by default there):
```
adb shell 'dd if=/dev/block/by-name/vendor_boot_a of=/sdcard/vendor_boot_a.stock.img bs=4M'
adb shell 'dd if=/dev/block/by-name/vendor_boot_b of=/sdcard/vendor_boot_b.stock.img bs=4M'
adb pull /sdcard/vendor_boot_a.stock.img
adb pull /sdcard/vendor_boot_b.stock.img
```

Sanity check: size must be 67108864 bytes (64 MB). Flash it back with:
```
fastboot flash vendor_boot_a vendor_boot_a.stock.img   # or _b
```

## 12. Fastboot/adb cheat sheet

```
fastboot getvar current-slot
fastboot --set-active=a|b
fastboot flash vendor_boot_a|b <file>
fastboot flash vendor_boot_a: <ramdisk.lz4>   # Pixel 6 series only (note the colon)
fastboot fetch vendor_boot_a|b ./backup.img   # backup without root
fastboot reboot recovery
fastboot reboot
adb devices -l
adb pull /tmp/recovery.log
adb pull /tmp/weaver.log
adb shell 'dmesg > /tmp/dmesg.log; logcat -d -v time > /tmp/logcat.log'
adb pull /tmp/dmesg.log
adb pull /tmp/logcat.log
adb shell 'dmesg | grep -i -E "touch|keymint|trusty|gsc|sg1" | head -30'
```
