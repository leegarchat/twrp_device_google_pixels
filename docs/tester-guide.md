# Testing OrangeFox test builds - full guide

> [Русская версия](tester-guide_ru.md)

`Guide v3 · 2026-10-26 · field protocol`

For testers of **experimental (test) builds**. Test builds can fail to boot, break decryption, or leave a slot unbootable - follow this guide exactly and you will always have a way back.

> Test builds: **never Format Data unless asked**. Flash **only** the partition you were told to (normally `vendor_boot_X`). Never flash `super`, `boot`, `system`, `modem`, bootloader or radio from a test package unless explicitly instructed.

## Contents

- [0. Prerequisites](#0-prerequisites)
- [1. Flashing a test build](#1-flashing-a-test-build)
- [2. First boot - the checklist](#2-first-boot--the-checklist)
- [3. Collecting logs - flog.sh first](#3-collecting-logs--flogsh-first)
- [4. Decryption test](#4-decryption-test)
- [5. How the UI renders](#5-how-the-ui-renders-folds-and-tablets-especially)
- [6. Outcomes A-D](#6-outcomes-ad)
- [7. Report template](#7-report-template-copy-paste)
- [8. Golden rules / backup / cheat sheet](#8-golden-rules-backup-cheat-sheet)

## 0. Prerequisites

- PC with `platform-tools` (`fastboot`, `adb`) working. USB cable + preferably a USB 2.0 port/hub (fewer flaky connections). Battery **> 50%**.
- **Factory images** for your exact device downloaded and unpacked (you need at least the factory `vendor_boot.img` for your slot) - or dump yours first ([§8](#8-golden-rules-backup-cheat-sheet)).
- Know your codename (`shiba`, `comet`, `grizzly`…) and current slot: `fastboot getvar current-slot`.
- Back up anything important.

<details>
<summary>✅ MD5 first - Telegram truncates files</summary>

- Verify the image hash **BEFORE** flashing. Telegram sometimes delivers `.img` files truncated (this really happened during R11 testing and cost days of pointless debugging). The maintainer posts an `md5` next to every test build (or pinned in https://t.me/OFRPforTensor) - compare: `md5sum OrangeFox-test-xxx.img` (Windows: `certutil -hashfile OrangeFox-test-xxx.img MD5`).
- Mismatch = broken download, **not** a bug. Re-download (preferably from the GDrive folder in the channel), never report it. Bootloop + empty pstore with a mismatched hash = broken download first.

</details>

## 1. Flashing a test build

The maintainer tells you **which file to flash and into which slot**. The default path is the **AIO installer package** below. Manual fastboot flows are legacy (pre-AIO) - use them only when explicitly told to.

<details>
<summary>📦 Step 0 - to unpack or not to unpack</summary>

- The download looks like `OrangeFox-<version>-<tag>-aio.zip` (tens of MB).
- **Desktop install (PC, below): UNPACK FIRST.** Windows: right-click → **Extract All…**. Linux: `unzip OrangeFox-*.zip -d ofox-install && cd ofox-install`. After unpacking you must see: `install-desktop.sh`, `install-desktop.bat`, `install-recovery.sh`, `export.txt`, `module.prop`, `customize.sh`, `bin/`, `META-INF/` and one `*.ramdisk.lz4` payload.
- **Magisk / KernelSU / recovery install: do NOT unpack.** Flash the `.zip` as-is.
- The #1 beginner mistake: running things from inside the archiver window. That never works. Nothing needs installing on the PC: platform-tools travel inside (`bin/`).

</details>

<details>
<summary>🖥️ Variant A - PC / Linux (from the bootloader)</summary>

1. Unpack the zip, open a terminal **in that folder**.
2. Phone into classic fastboot (bootloader): hold **Volume-Down + Power** (`FASTBOOT MODE` / `START` - NOT fastbootd). Or from system: `adb reboot bootloader`.
3. Connect USB, run `./install-desktop.sh` (or double-click `install-desktop.AppImage`).
4. Answer the arrow-key menus (slots - default `both` is safest). Watch: snapshot → backup → rebuild → flash → byte-compare.
5. Accept the offer to reboot to recovery. Backup lands in `backup/<date-time>/` (+ `install.log`). Success = `RESULT: OK`.

</details>

<details>
<summary>🪟 Variant B - PC / Windows (from the bootloader)</summary>

- Same as A, but double-click `install-desktop.bat`. First time ever: install the **Google USB driver** (`fastboot devices` empty without it). If the `.bat` flashes and closes - run it from `cmd` in that folder to read the error.

</details>

<details>
<summary>📲 Variant C - Magisk / KernelSU (booted, rooted system)</summary>

1. Do NOT unpack. Get the `.zip` onto the phone (download or MTP/`adb push`).
2. Magisk (or KernelSU) app → **Modules** → **Install from storage** → pick the zip → wait for install OK.
3. The module **deletes itself** afterwards - one-shot installer, nothing stays. Backup stays in `/sdcard/backup_vendor_boot/`.
4. Reboot to recovery yourself (Volume-Down + Power → **Recovery mode**, or `adb reboot recovery`). Needs: Android booted + root. `SLOT=` in `export.txt` decides the slots (default `both`).

</details>

<details>
<summary>🔄 Variant D - from recovery (no PC, no root needed)</summary>

1. Do NOT unpack. Get the `.zip` onto the phone (MTP/`adb push`/Downloads).
2. Any recovery (TWRP/OrangeFox) → **Install** → pick the zip → swipe. Flashing the recovery you sit in is fine - it runs from RAM.
3. Watch for `RESULT: OK` + backup path (`/sdcard/backup_vendor_boot/` or `/tmp/recovery_install/`).
4. **Reboot → Recovery** (not System) to boot straight into the new build.

</details>

<details>
<summary>📼 Legacy: manual fastboot flashing (pre-AIO only)</summary>

- Default (example slot A): `fastboot flash vendor_boot_a OrangeFox-test-xxx.img` → `fastboot reboot recovery`.
- **Pixel 6 series (gs101) special - pre-AIO builds only** (`test_1-gs101`): no `vendor_kernel_boot` there, so flash the `.ramdisk.lz4` into the platform fragment (note the trailing colon = empty fragment name): `fastboot flash vendor_boot_a: OrangeFox-test-xxx-gs101.ramdisk.lz4`. `:default` or `:recovery` are WRONG - the first collapses the table and kills stock dlkm, the second overwrites the wrong fragment.
- For `-aio` builds, **NEVER** raw-flash the cpio with fastboot - the image is family-neutral and needs the installer swap (USB controller, fstab, KeyMint manifests) first. Raw-flashed AIO on Pixel 6 boots with broken USB (no adb) and wrong geometry. Use the AIO installer above.

</details>

After any variant: wait for the recovery main screen, then continue. Keep the `backup/` (PC) or `/sdcard/backup_vendor_boot/` copy until testing is over - that is your rollback. Config: `export.txt` (`SLOT=both|a|b|current`, `MIN_FREE_MB=7` strict). Logs: `backup/<date-time>/install.log` (PC) or `/tmp/recovery_install/install.log` (on-device).

## 2. First boot - the checklist

After `fastboot reboot recovery`, wait for the main screen (USB stack and decryption services start with a delay), then:

> **adb on the password screen (non-Stable test builds).** You received a NON-Stable (Beta) build - it does not force-disable adb/MTP like Stable. So adb on the initial PIN prompt **may work or may not - the developer genuinely does not know** (their own device is currently unencrypted, so they cannot check this path). Your job: record the fact. `adb devices` sees it → run everything right there, report **present**. `no devices` → leave the password screen (PIN if touch works, or **Skip** / back key to main without decrypting - may be absent; then PIN is the only way out), run after, report **absent**. Logs are written from the very start either way - pulling them later loses nothing.

```bash
adb devices -l
adb shell 'getprop sys.usb.config; getprop init.svc.vendor.keymint.rust-trusty; getprop init.svc.vendor.keymint-trusty; uname -r; getprop ro.boot.slot_suffix'
```

| Check | Healthy value |
|---|---|
| `adb devices` | device listed as `recovery` |
| `sys.usb.config` | `mtp,adb` |
| KeyMint service | `running` (exactly one of rust-trusty / trusty) |
| `uname -r` | kernel the build was made for (ask maintainer if unsure) |
| slot suffix | the slot you flashed (`_a` / `_b`) |

Then collect the logs - §3, `flog.sh` first. One command replaces all manual pulls below.

## 3. Collecting logs - flog.sh first

`flog.sh` rides inside every current test build. One command collects the whole bundle - **prefer it over manual `adb pull`**: strictly more (early-boot AIO logs, props, backlight state, pstore, installer logs) + world-readable permissions so MTP/PC readers open every file.

From recovery - over adb **or** Fox terminal (Advanced → Terminal, no adb needed): `adb shell flog.sh`. Output ends with the bundle location (remember the timestamp): `flog: DONE -> /data/media/0/fox_logs/fox_logs_<device>_<family>_<stamp>.tar.gz` (when userdata is locked the DONE line is a bare `/tmp/fox_logs/<stamp>/` directory instead - pull it immediately, RAM dies on reboot).

<details>
<summary>💾 Where the bundle lands - userdata matrix</summary>

| userdata state | Destination | How to get it to the PC |
|---|---|---|
| Decrypted + writable (PIN entered, or no encryption) | `/data/media/0/fox_logs/<stamp>/` - **survives reboot** | `adb pull` any time later (even from booted system), or copy via MTP |
| Locked / unwritable | `/tmp/fox_logs/<stamp>/` - RAM, **dies on reboot** | `adb pull` **immediately**, while still in recovery |

</details>

<details>
<summary>📋 What the bundle contains</summary>

| File | Source | Proves |
|---|---|---|
| `recovery.log`, `weaver.log` | `/tmp/*.log` | recovery + decrypt flow |
| `reflash_twrp.log`, `install-*.log` | on-device install | reflash/install path (if used) |
| `aio_stub.log`, `aio_runatinit.log` | stub + Rust early-boot | family detection, swap, USB rescue |
| `dmesg.log`, `logcat.log` | kernel + system | touch/display/USB/decrypt hardware |
| `props.txt` | `getprop` + key props | slot, family, services, USB state |
| `backlight.txt`, battery+health snapshot | backlight sysfs, health HAL | screen + power issues while broken |
| `pstore*` | `/sys/fs/pstore/` | kernel crashes (survive reboot) |
| `families.txt`, `pixelrunatboot.json` | installer config | which payload/slot policy produced this boot |

`- missing:` lines are also an answer (absent `aio_stub.log` = the stub never logged). Do not delete them; send the console output too. For hardware debugging (touch, display, USB, decrypt, sensors) `dmesg`/`logcat` matter most after `recovery.log` - always full files, never excerpts.

</details>

<details>
<summary>🆘 No adb at all (dead USB) / manual fallback</summary>

- No adb: Fox terminal → `flog.sh` → bundle lands on userdata if decrypted → reboot to system, pull via `adb`/MTP. If userdata locked too - photograph key screens, go to Outcome B/C.
- Manual fallback (builds without `flog.sh`):

```bash
adb pull /tmp/recovery.log
adb pull /tmp/weaver.log
adb shell 'dmesg > /tmp/dmesg.log; logcat -d -v time > /tmp/logcat.log'
adb pull /tmp/dmesg.log
adb pull /tmp/logcat.log
```

</details>

## 4. Decryption test

1. If the device asks for PIN/password/pattern - enter it.
2. Success = you see your files (Internal Storage) and `recovery.log` contains `User 0 Decrypted`.
3. If touch does not work: **that is NOT a test failure** unless the maintainer said otherwise. Report it, continue over `adb`.
4. Send back: the `flog.sh` bundle (§3) + whether `/data/media` is visible.

## 5. How the UI renders - folds and tablets especially!

1. **Which screen shows the UI** (fold: cover/outer or inner? Folded or unfolded at boot?).
2. **Black bars**: present? Which sides? Bars are the normal letterbox mode - presence/absence + symmetry matter.
3. **Stretching**: circles vs ellipses? Same scale for text/images/frames, or "floating"? Keyboard full width or narrower?
4. **Rotation**: upright or 90°? (Fold inner may rotate - which way?)
5. **Touch-to-screen match**: taps land where shown? (Fold: outer touch works on outer, not vice versa?)
6. **Photograph the screen** with another phone - a photo beats words. Fold: photos folded + unfolded.

If adb is available, add: `adb shell 'getprop DOF_SCREEN_W; getprop DOF_SCREEN_H; getprop DOF_PROGRESSIVE_SCALE'`.

<details>
<summary>🧪 Feature tests (only if asked)</summary>

- **Touch**: works / partial / dead + `dmesg | grep -i touch`. **Torch / haptics**: works or not. **OTG / USB mouse**: works or not - with a drive attached and the PC unplugged, run `otg_test.sh` (Fox terminal) for the T1-T8 verdicts, attach `otg_test_<device>_<family>_<stamp>.tar.gz`.
- **Format Data / Reflash Recovery: FORBIDDEN on test builds** unless the maintainer explicitly allows it (some builds refuse reflash by design - report the refusal text, it confirms a guard works).

</details>

## 6. Outcomes A-D

**Outcome A - boots, adb works:** do §2-5, send the report (§7). Done.

<details>
<summary>🅱️ Outcome B - boots, but NO adb</summary>

1. Wait a full 60 s, unplug/replug, try another port / USB 2.0 hub.
2. Then logs without adb: Fox terminal → `flog.sh` (§3).
3. Check fastbootd: `fastboot devices`. Visible there = kernel alive, USB-gadget problem - report it.
4. Completely silent → go to Outcome C (rollback).

</details>

<details>
<summary>🅲 Outcome C - does NOT boot (logo / black / bootloop)</summary>

**Do not panic. Do not flash random things.** Roll back:

1. Flash the **factory (known-good)** `vendor_boot.img` into the **same slot**: `fastboot flash vendor_boot_a stock_vendor_boot.img` (or `_b`).
2. Boot to system normally.
3. With root, capture kernel crash dumps **before they are overwritten** (a second boot may rotate them - one boot only):

```bash
su
ls /sys/fs/pstore/
cat /sys/fs/pstore/console-ramoops-0 > /sdcard/ramoops_console.txt
cat /sys/fs/pstore/dmesg-ramoops-0  > /sdcard/ramoops_dmesg.txt
dmesg > /sdcard/dmesg_boot.txt
getprop ro.boot.slot_suffix; uname -r
```

4. Pull to PC (`adb pull /sdcard/ramoops_console.txt` …), send **all of them** + which file was flashed into which slot.

**Boot → crash → switch-slot trick:** if slot B bootloops, the bootloader may mark it unbootable and fall back to slot A - check `fastboot getvar current-slot` after the failure. Or force the other slot (`fastboot --set-active=a` / `b`), boot to system, collect pstore as above. Key point: **pstore survives a reboot but not many** - collect on the FIRST successful boot after the crash, do not reboot twice.

</details>

<details>
<summary>🅳 Outcome D - boots with adb, but decryption FAILS</summary>

Send the `flog.sh` bundle (§3) plus:

1. Manual HAL run (~15 s, then Ctrl-C): `adb shell '/vendor/bin/hw/android.hardware.security.keymint-service.rust.trusty --dev /dev/trusty-ipc-dev0'` (C++ HAL builds: the maintainer gives another path).
2. Service states: `adb shell 'getprop init.svc.recovery_storageproxyd; getprop init.svc.recovery_weaver'`.

</details>

## 7. Report template (copy-paste)

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
Attached   : fox_logs/<stamp>/ (whole flog.sh bundle: recovery.log, weaver.log,
             dmesg.log, logcat.log, aio_*.log, props.txt, …)
             - or the same files pulled manually
             (+ ramoops_console.txt, ramoops_dmesg.txt, dmesg_boot.txt if Outcome C)
Notes      : <anything unusual: how long boot took, error texts, ...>
```

## 8. Golden rules, backup, cheat sheet

<details>
<summary>🏆 Golden rules</summary>

1. One change at a time. Never combine a test build with other mods.
2. Factory (known-good) images ready **before** flashing, not after.
3. Logs first, conclusions later - always attach full files.
4. Do NOT press "Reflash Recovery", do NOT Format Data, do NOT flash other partitions on test builds unless told to.
5. If something behaves unexpectedly - photograph/write down the exact text. "It didn't work" is not a report.

</details>

<details>
<summary>💾 How to dump your factory vendor_boot (backup before testing)</summary>

No factory images at hand? Dump the known-good `vendor_boot` from the device **before** flashing. Easiest - bootloader, no root needed (works for Pixel 6 too):

```bash
fastboot fetch vendor_boot_a ./vendor_boot_a.stock.img
fastboot fetch vendor_boot_b ./vendor_boot_b.stock.img
```

Rooted system (both slots at once):

```bash
su
dd if=/dev/block/by-name/vendor_boot_a of=/sdcard/vendor_boot_a.stock.img bs=4M
dd if=/dev/block/by-name/vendor_boot_b of=/sdcard/vendor_boot_b.stock.img bs=4M
```

From recovery (adb, root by default there):

```bash
adb shell 'dd if=/dev/block/by-name/vendor_boot_a of=/sdcard/vendor_boot_a.stock.img bs=4M'
adb shell 'dd if=/dev/block/by-name/vendor_boot_b of=/sdcard/vendor_boot_b.stock.img bs=4M'
adb pull /sdcard/vendor_boot_a.stock.img
adb pull /sdcard/vendor_boot_b.stock.img
```

Sanity check: size must be **67108864 bytes (64 MB)**. Flash back: `fastboot flash vendor_boot_a vendor_boot_a.stock.img` (or `_b`).

</details>

<details>
<summary>⌨️ Fastboot/adb cheat sheet</summary>

```bash
fastboot getvar current-slot
fastboot --set-active=a|b
fastboot flash vendor_boot_a|b <file>   # legacy manual flow (pre-AIO)
fastboot flash vendor_boot_a: <ramdisk.lz4>   # legacy, Pixel 6 series only (note the colon)
./install-desktop.sh                              # AIO installer, Linux (from bootloader)
install-desktop.bat                               # AIO installer, Windows (from bootloader)
fastboot fetch vendor_boot_a|b ./backup.img   # backup without root
fastboot reboot recovery
fastboot reboot
adb devices -l
adb shell flog.sh                                 # full report bundle (preferred)
adb pull /data/media/0/fox_logs/<stamp>          # pull the bundle (userdata)
adb pull /tmp/fox_logs/<stamp>                   # pull the bundle (no userdata, urgent)
adb pull /tmp/recovery.log
adb pull /tmp/weaver.log
adb shell 'dmesg > /tmp/dmesg.log; logcat -d -v time > /tmp/logcat.log'
adb pull /tmp/dmesg.log
adb pull /tmp/logcat.log
adb shell 'dmesg | grep -i -E "touch|keymint|trusty|gsc|sg1" | head -30'
```

</details>
