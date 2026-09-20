# Testing OrangeFox test builds — full guide (EN)

This guide is for testers of **experimental (test) OrangeFox builds**.
Test builds can fail to boot, break decryption, or leave a slot unbootable —
follow this guide exactly and you will always have a way back.

## Contents

- [0. Prerequisites](#0-prerequisites)
- [1. Flashing a test build](#1-flashing-a-test-build)
- [2. First boot — the 60-second checklist](#2-first-boot--the-60-second-checklist)
- [3. Collecting logs — flog.sh first](#3-collecting-logs--flogsh-first)
- [4. Decryption test](#4-decryption-test)
- [5. How the UI renders](#5-how-the-ui-renders-folds-and-tablets-especially)
- [6. Outcomes A–D](#6-outcomes-ad)
- [7. Report template](#7-report-template-copy-paste)
- [8. Golden rules](#8-golden-rules)
- [9. How to dump your factory vendor_boot](#9-how-to-dump-your-factory-vendor_boot-backup-before-testing)
- [10. Fastboot/adb cheat sheet](#10-fastbootadb-cheat-sheet)

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
The default path for all current builds is the **AIO installer package**
below. Manual fastboot flows are legacy (pre-AIO) and live under the
spoiler at the end of this section — use them only when the maintainer
explicitly says so.

### AIO installer package (recommended)

For AIO test builds the maintainer ships an installer package instead
of a raw `.img`: the recovery payload (`OrangeFox-*-aio.ramdisk.lz4`),
`export.txt`, installer binaries and bundled platform-tools. It does a
smart replace — your stock `vendor_boot` is fetched, only the recovery
fragment is swapped in, both slots are rebuilt + byte-verified, and a
dated backup is kept (`backup/` on PC, `/sdcard/backup_vendor_boot/`
on-device). First_stage, kernel cmdline and dlkm always stay stock.

Pick one variant (all perform the same install — the phone ends up with
OrangeFox in recovery; only the starting point differs):

#### Step 0 — to unpack or not to unpack

The downloaded file looks like `OrangeFox-R12.0-test6-aio.zip` (~47 MB).

- **Desktop install (Variants A/B below): UNPACK FIRST.** You need real
  files on disk, not an archive preview.
  - Windows: right-click the zip → **Extract All…** into a folder.
  - Linux: `unzip OrangeFox-*.zip -d ofox-install && cd ofox-install`.
  - After unpacking you must see: `install-desktop.sh`,
    `install-desktop.bat`, `install-recovery.sh`, `export.txt`,
    `module.prop`, `customize.sh`, `bin/`, `META-INF/` and one
    `*.ramdisk.lz4` payload. If you don't see these — you didn't unpack.
  - ⚠️ The #1 beginner mistake: double-clicking the zip and running
    things from inside the archiver window. That never works.
- **Magisk / KernelSU / recovery install (Variants C/D below): do NOT
  unpack.** Flash the downloaded `.zip` file as-is.

Nothing needs installing on the PC: platform-tools travel inside the
package (`bin/`), the scripts use the bundled ones.

#### Variant A — PC / Linux (from the bootloader)

1. Unpack the zip (see Step 0), open a terminal **in that folder**.
2. Put the phone into classic fastboot (bootloader) mode: power off,
   then hold **Volume-Down + Power** until the fastboot screen appears
   (it says `FASTBOOT MODE` / `START` — NOT fastbootd, which looks like
   a recovery menu). Alternative from a booted system:
   `adb reboot bootloader`.
3. Connect the USB cable and run:
   ```
   ./install-desktop.sh
   ```
   (or double-click `install-desktop.AppImage`).
4. Answer the arrow-key menus (which slots — default `both` is the
   safest; it keeps a backup). Watch the lines: snapshot → backup →
   rebuild → flash → byte-compare proof.
5. At the end accept the offer to reboot to recovery.

Backup lands in `backup/<date-time>/` next to the script (with
`install.log` inside). Success = `RESULT: OK` on screen.

#### Variant B — PC / Windows (from the bootloader)

Same as Variant A, but after unpacking just double-click
`install-desktop.bat` (a console window opens). Notes for Windows:

- First time ever: install the **Google USB driver** or the phone will
  not appear in fastboot (`fastboot devices` empty).
- If the `.bat` flashes and closes instantly — run it from `cmd` in that
  folder to see the error text.
- The end-of-install reboot-to-recovery offer works the same.

#### Variant C — Magisk / KernelSU (from a booted, rooted system)

1. Do NOT unpack. Get the `.zip` onto the phone (download it there or
   copy via MTP/`adb push`).
2. Open the Magisk (or KernelSU) app → **Modules** → **Install from
   storage** → pick the zip → wait until it prints install OK.
3. The module **deletes itself** afterwards — nothing stays installed,
   this was a one-shot installer (backup stays in
   `/sdcard/backup_vendor_boot/`).
4. Reboot to recovery yourself: hold **Volume-Down + Power** → choose
   **Recovery mode** in the bootloader menu (or `adb reboot recovery`).

Requirements: Android actually booted + root (that's what the manager
is for). `SLOT=` in `export.txt` decides the slots (default `both`).

#### Variant D — from recovery (no PC, no root needed)

1. Do NOT unpack. Get the `.zip` onto the phone (MTP/`adb push`/
   downloaded, e.g. into Downloads).
2. In any recovery (TWRP/OrangeFox) → **Install** → pick the zip →
   swipe to confirm. Flashing the recovery you are currently sitting in
   is fine — it runs from RAM.
3. Watch the console for `RESULT: OK` and the backup path
   (`/sdcard/backup_vendor_boot/` or `/tmp/recovery_install/`).
4. **Reboot → Recovery** (not System) to boot straight into the new build.

#### After any variant

- First boot to recovery can take up to 60 seconds — continue with
  [§2](#2-first-boot--the-60-second-checklist).
- Keep the `backup/` (PC) or `/sdcard/backup_vendor_boot/` (device)
  copy until testing is over — that is your rollback.

Config lives in `export.txt` next to the installer:
`SLOT=both|a|b|current`, `MIN_FREE_MB=7` (strict — the install aborts
instead of flashing an over-full partition). Logs:
`backup/<date-time>/install.log` (PC) or
`/tmp/recovery_install/install.log` (on-device).

Use the manual `fastboot flash vendor_boot_X` flow from the legacy spoiler
below only when the maintainer explicitly says so.

<details>
<summary>Legacy: manual fastboot flashing (pre-AIO builds only — click to expand)</summary>

Default procedure (example: slot A):

```
fastboot flash vendor_boot_a OrangeFox-test-xxx.img
fastboot reboot recovery
```

#### Pixel 6 series (gs101: oriole/raven/bluejay) — special procedure

> ⚠️ APPLIES TO PRE-AIO BUILDS ONLY (e.g. `test_1-gs101`). For `-aio`
> builds, NEVER flash the raw cpio with fastboot — the image is
> family-neutral and needs the installer swap (USB controller, fstab,
> keymint manifests) before it can boot correctly on gs101. Use the AIO
> installer package above; raw-flashed AIO on Pixel 6 boots with broken
> USB (no adb) and wrong display geometry.

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

</details>

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

Then collect the logs — [see §3](#3-collecting-logs--flogsh-first), `flog.sh`
first. One command replaces all manual pulls below.

---

## 3. Collecting logs — flog.sh first

`flog.sh` rides inside every current test build. One command collects the
whole report bundle — **prefer it over manual `adb pull`**: it grabs
strictly more (early-boot AIO logs, props, backlight state, pstore,
installer logs) and forces world-readable permissions so MTP/PC readers
can open every file.

Run it from recovery — over adb **or** from the Fox terminal
(Advanced → Terminal — no adb needed at all):

```
adb shell flog.sh
```

Output ends with the bundle location (remember the timestamp):

```
flog: DONE -> /data/media/0/fox_logs/20260920_110538
```

### 3.1. Where the bundle lands — userdata matrix

| userdata state | destination | how to get it to the PC |
|---|---|---|
| Decrypted + writable (PIN entered, or no encryption) | `/data/media/0/fox_logs/<stamp>/` — **survives reboot** | `adb pull /data/media/0/fox_logs/<stamp>` any time later (even from booted system), or copy via MTP |
| Locked / unwritable | `/tmp/fox_logs/<stamp>/` — RAM, **dies on reboot** | `adb pull` **immediately**, while still in recovery |

### 3.2. What the bundle contains

| File | Source | Proves |
|---|---|---|
| `recovery.log`, `weaver.log` | `/tmp/*.log` | recovery + decrypt flow |
| `reflash_twrp.log`, `install-*.log` | on-device install | reflash/install path (if used) |
| `aio_stub.log`, `aio_runatinit.log` | stub + Rust early-boot | family detection, swap, USB rescue |
| `dmesg.log`, `logcat.log` | kernel + system | touch/display/USB/decrypt hardware |
| `props.txt` | `getprop` + key props | slot, family, services, USB state |
| `backlight.txt` | backlight sysfs | screen issues (values while broken) |
| `pstore*` | `/sys/fs/pstore/` | kernel crashes (survive reboot) |
| `families.txt`, `pixelrunatboot.json` | installer config | which payload/slot policy produced this boot |

`- missing:` lines in the `flog.sh` output are also an answer — an absent
`aio_stub.log`, for example, tells the developer the stub never logged.
Do not delete them; send the console output too.

### 3.3. No adb at all (dead USB)?

1. Fox terminal → `flog.sh` → bundle lands on userdata if decrypted.
2. Reboot to system, pull via `adb` or copy via MTP.
3. If userdata is locked too — photograph the key screens and go to
   [Outcome B/C](#6-outcomes-ad).

### 3.4. Manual fallback (builds without flog.sh)

```
adb pull /tmp/recovery.log
adb pull /tmp/weaver.log
adb shell 'dmesg > /tmp/dmesg.log; logcat -d -v time > /tmp/logcat.log'
adb pull /tmp/dmesg.log
adb pull /tmp/logcat.log
```

For any hardware-level debugging (touch, display, USB, decrypt, sensors)
`dmesg`/`logcat` are the most important files after `recovery.log`
itself — always full files, never excerpts.

---

## 4. Decryption test

1. If the device asks for PIN/password/pattern — enter it.
2. Success = you see your files (Internal Storage) and `recovery.log`
   contains `User 0 Decrypted`.
3. If touch does not work: **that is NOT a test failure** unless the
   maintainer said otherwise. Report it, continue over `adb`.
4. Send back: the `flog.sh` bundle ([§3](#3-collecting-logs--flogsh-first)),
   and whether `/data/media` is visible.

---

## 5. How the UI renders (folds and tablets especially!)

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

### 5.1. Feature tests (only if asked)

- **Touch**: works / partially / dead + `dmesg | grep -i touch`.
- **Torch / haptics**: works or not.
- **OTG / USB mouse**: works or not.
- **Format Data / Reflash Recovery**: **FORBIDDEN on test builds**
  unless the maintainer explicitly allows it (some test builds refuse
  reflash by design — report the refusal text, it confirms a guard works).

---

## 6. Outcomes A–D

### Outcome A — boots, adb works

Do sections 2–5 and send the report ([see §7](#7-report-template-copy-paste)).
You are done.

### Outcome B — boots, but NO adb

1. Wait a full 60 s, unplug/replug the cable, try another port / USB 2.0 hub.
2. Then collect logs without adb: Fox terminal → `flog.sh`
   ([§3.3](#33-no-adb-at-all-dead-usb)).
3. Check fastbootd: `fastboot devices`. If the device is visible in
   fastbootd, the kernel is alive and it is a USB-gadget problem — report it.
4. If completely silent, go to Outcome C (rollback).

### Outcome C — does NOT boot (logo / black screen / bootloop)

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

#### The boot → crash → switch-slot / boot-to-system trick

- If slot B with the test build bootloops, the bootloader may mark it
  unbootable and fall back to slot A by itself — check
  `fastboot getvar current-slot` after the failure.
- You can also force the other slot: `fastboot --set-active=a` (or `b`),
  then boot to system and collect the pstore dumps as above.
- Key point: **pstore survives a reboot but not many** — always collect
  dumps on the FIRST successful boot after the crash, do not reboot twice.

### Outcome D — boots with adb, but decryption FAILS

Send the `flog.sh` bundle ([§3](#3-collecting-logs--flogsh-first)) plus:

1. Output of a manual HAL run (run ~15 seconds, then Ctrl-C):
   ```
   adb shell '/vendor/bin/hw/android.hardware.security.keymint-service.rust.trusty --dev /dev/trusty-ipc-dev0'
   ```
   (If your build uses the C++ HAL, the maintainer will give another path.)
2. Service states:
   ```
   adb shell 'getprop init.svc.recovery_storageproxyd; getprop init.svc.recovery_weaver'
   ```

---

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
             — or the same files pulled manually
             (+ ramoops_console.txt, ramoops_dmesg.txt, dmesg_boot.txt if Outcome C)
Notes      : <anything unusual: how long boot took, error texts, ...>
```

---

## 8. Golden rules

1. One change at a time. Never combine a test build with other mods.
2. Factory (known-good) images ready **before** flashing, not after.
3. Logs first, conclusions later — always attach full files.
4. Do NOT press “Reflash Recovery”, do NOT Format Data, do NOT flash
   other partitions on test builds unless told to.
5. If something behaves unexpectedly — photograph/write down the exact
   text. “It didn't work” is not a report.

## 9. How to dump your factory vendor_boot (backup before testing)

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

## 10. Fastboot/adb cheat sheet

```
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
