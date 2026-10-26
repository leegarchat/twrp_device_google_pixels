# OrangeFox for Tensor Pixel - `device/google/pixels`

[English](README.md) | **[Русская версия](README_ru.md)**

`Guide v3 · 2026-10-26 · R12 / AIO universal`

One installer zip for **all Tensor Pixels** - Pixel 6 → 11, folds, tablet (25 devices, 6 SoC families). The exact model is detected at runtime, stock kernel is always kept, only the recovery ramdisk is delivered. Recovery lives in `vendor_boot` only.

<details>
<summary>⚠️ Read first - what this is and isn't</summary>

- This is **recovery**, not a ROM: a mini-system for flashing zips, backups, formatting. It does not replace Android.
- Installing it needs an **unlocked bootloader - which wipes your phone** (photos, chats, files). Back up first.
- **Don't start if:** this is your only phone and you need it tomorrow; you never used a PC command line. Difficulty: **medium**. Time: ~10 min. The first boot into recovery takes a little longer than usual - wait for the main screen.
- 100% boot guarantee - only on `shiba` (maintainer's daily driver: every build boots there first, before it goes anywhere else). Other families' cells below are field/tester results - green means confirmed on real hardware, not guessed. New here? → [tester guide](docs/tester-guide.md).

</details>

<details>
<summary>🔥 Why this build</summary>

- **AIO universal** - one `*-aio.zip` covers every device; no per-model images to mix up.
- **Stock kernel kept** - no kernel profiles, no cmdline surgery. Your `vendor_boot` layout, first-stage, dlkm stay stock; only the recovery part is replaced.
- **Decrypts data** - FBE metadata via Trusty TEE KeyMint built from source (no prebuilt HALs), Titan M2/M3 weaver (M2 on Pixel 6-9, M3 on 10/11), zoned+exp multi-device.
- **Touch that survives** - modules are tried from your stock `vendor_dlkm` first; if `super` is broken and nothing reads from there, the fallback is the archive the installer injected (`modules/<device>.lgz`). Broken `super`? Touch still works (verified live on G3: stock unreadable on both slots, 12/12 modules load).
- **Fool-proof USB** - kernel-vs-firmware patch check before any VBUS switch. Mismatch = plain device mode (adb safe) instead of a panic.
- **Fast flash + logs** - optional High-speed ROM install (local ~1 min vs ~4 stock, sideload supported too), Live slot switch (a ROM → GApps → Magisk queue flashes in one go, no reboot), one-command log bundle `flog.sh`.

</details>

<details>
<summary>📱 Devices (25) - one zip for all</summary>

| Family | SoC | Devices |
|---|---|---|
| `gs101` | Tensor G1 | oriole (6), raven (6 Pro), bluejay (6a) |
| `gs201` | Tensor G2 | cheetah (7 Pro), panther (7), lynx (7a), felix (Fold), tangorpro (Tablet) |
| `zuma` | Tensor G3 | shiba (8), husky (8 Pro), akita (8a) |
| `zumapro` | Tensor G4 | tokay (9), caiman (9 Pro), komodo (9 Pro XL), tegu (9a), stallion (10a), comet (9 Pro Fold) |
| `laguna` | Tensor G5 | frankel (10), blazer (10 Pro), mustang (10 Pro XL), rango (10 Pro Fold) |
| `malibu` | Tensor G6 | cubs (11), grizzly (11 Pro), kodiak (11 Pro XL), yogi (11 Pro Fold) |

Each device's vendor specifics (touch modules, UFS/USB paths, props, KeyMint type) live in JSON (`families/*/family.json`, `devices/*/pixel.json`), resolved at boot - nothing is baked per model.

</details>

## Install - the main path (Type A, AIO zip)

<details>
<summary>☑️ Before you start</summary>

- Bootloader **unlocked**. Battery 50%+, original cable, USB 2.0 port if fastboot is flaky.
- **Unpack the zip first** (PC install): you need real files - `install-desktop.sh` / `install-desktop.bat`, `export.txt`, `bin/`, one `*.ramdisk.lz4`. Running from inside the archiver window never works.
- **Check MD5** before flashing: Linux `md5sum file`, Windows `certutil -hashfile file MD5`. Mismatch = broken download, re-download, never report it.
- First time on Windows: install the **Google USB driver**, or fastboot won't see the phone.

</details>

1. Phone in **bootloader** mode (Volume Down + Power), connect USB.
2. Run the installer: Linux `./install-desktop.sh`, Windows `install-desktop.bat` (or double-click the AppImage / `.exe`).
3. It fetches your stock `vendor_boot`, replaces **only the recovery part**, rebuilds **both slots**, byte-checks the flash. Success = `RESULT: OK`. The installer asks which layout you want: **Type A** (classic, proven - everyone except Pixel 6 picks this), **Type B** (all-in-platform), **Type C** (payload-only platform). B and C are experiments; Pixel 6 testers try all three in a loop (stock `vendor_boot` → install → report recovery-boot? system-boot? → repeat) because normal boot is broken there and the layouts are the fix candidates.
4. **Immediately** reboot to recovery (`fastboot reboot recovery`), don't boot system yet. The first boot takes a little longer than usual - wait for the main screen.
5. Done - OrangeFox is in `vendor_boot` on both slots, stock kernel/first-stage untouched. Backup + `install.log` are in `backup/<date-time>/` next to the script.

How the installer picks files: your codename is auto-detected (`ro.product.device` → `ro.build.product`, validated against the Pixel pool in the binary); the matching touch archive `modules/<device>.lgz` is injected into the payload root (`--inject-lgz`), the target family's KeyMint HAL + VINTF manifest are kept while siblings are dropped (`ro.recovery.keymint` enforced), and the USB controller address is substituted into the rc files. Small installer binary is preferred, arch auto-detected. Layout override: `RECOVERY_IS_PLATFORM=` in `export.txt` - empty = Type A classic, `var1` = all-in-platform, `var2` = payload-only platform.

> **On stock + Windows? Do this path and ignore the rest.** The ways below need root / recovery / a second phone you don't have yet.

<details>
<summary>📦 Other ways to install the same zip (5 total)</summary>

- **Magisk / KernelSU / APatch** - flash the zip in the manager app like a module. It is **not** a module: install is permanent, it deletes itself after.
- **Any recovery (OFox / TWRP / Lineage)** - Install → pick the zip → swipe. In Lineage recovery: ADB Sideload → `adb sideload file.zip`.
- **ADB sideload** - in recovery Advanced → ADB Sideload, then `adb sideload file.zip` on PC.
- **Termux (phone-to-phone over OTG)** - target via USB-OTG, same as Linux (`./install-desktop.sh`), but first `pkg install android-tools`.
- **Terminal / adb shell (rooted system)** - if the running system already has root.
- ARM64 PCs / Termux: bundled tools are x64-only - install platform-tools system-wide first. 32-bit systems not supported.

</details>

## Flashing ROMs - high-speed + live slot (in recovery)

- **High-speed flash** (off by default): a ROM zip installs in ~1 min - field-measured local 0:50 vs 3:50 stock, sideload 3:05 vs 8:30 stock (3.3 GB zip, shiba). Toggle on the flash-confirm page, and on the sideload screen for sideload installs. Works on encrypted devices too. Details → [engine doc](docs/recovery-engine.md).
- **Live slot switch** (off by default): no reboot between ROM and addons - a **ROM → GApps → Magisk** queue flashes in one go. Details → [engine doc](docs/recovery-engine.md).

## Update / reinstall - from inside recovery

- No PC needed: snapshots of both `vendor_boot` slots are rebuilt from each slot's **own stock image** (header, cmdline, dtb come from the slot itself, nothing is stamped).
- Backups live at `/sdcard/backup_vendor_boot/` (+ `install.log`). **Copy the backup off the phone BEFORE any ROM Format Data** - Format wipes `/sdcard` too. `adb pull` it to PC first.
- To restore: installer → Restore → pick backup. Restored bytes are flashed as-is.

## What works - field status

> 🟢 works (confirmed) · 🔴 broken · 🟡 not tested · 🔵 should work, not verified · 🟠 flaky

| Feature | G1 | G2 | G3 | G4 | G5 | G6 |
|---|---|---|---|---|---|---|
| Recovery boots | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Data decryption | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| MTP + ADB | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| ROM flashing | 🟢 | 🟢 | 🟢 | 🟢 | 🔵 | 🟢 |
| ADB sideload | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Backup / restore | 🟢 | 🟢 | 🟢 | 🟢 | 🔵 | 🟢 |
| Format data | 🟢 | 🟢 | 🟢 | 🟢 | 🔵 | 🟢 |
| OTG | 🟢 | 🔵 | 🟢 | 🟢 | 🟢 | 🟡 |
| Flashlight | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🔵 |
| Vibration | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Touchscreen | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Touch (broken super) | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Vibration (broken super) | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 | 🔴 |
| Brightness control | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Reboot to Android | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Reboot menu | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| PIN unlock | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Slot switch | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Fastbootd | 🟢 | 🔵 | 🟢 | 🟢 | 🔵 | 🟢 |
| Files + terminal | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| OTA survival | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Charging | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |
| Self-reflash | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 | 🟢 |

<details>
<summary>🔍 Footnotes - broken super, USB guard, testers</summary>

- **Broken `super`:** touch works because the modules come from the archive the installer placed into the payload (verified live on G3: stock fails on both slots, 12/12 modules load). No vibration - the haptics firmware lives on `vendor` inside `super`, which is unreadable then.
- **Fool-proof USB guard:** a newer-than-firmware kernel panics when the custom OTG stack force-switches VBUS to host (seen in the field). Before any shim load / power write, recovery compares the running kernel's patch level against firmware modules' `vermagic` (majority over stock `.ko`; own shims excluded). Mismatch = whole custom stack stays off (no services, no VBUS switch - plain device mode, adb safe). Monolithic kernels skip the check; trees with no readable stock modules fail open with a loud log line.
- Testers are not very active - minor per-family fixes are on hold; the current state is stable enough for a first release.

</details>

<details>
<summary>❓ Questions → answers</summary>

- **Which file do I flash?** → The `*-aio.zip` installer for your build. No per-device images exist.
- **Unpack or not?** → PC install: **unpack first**. Manager / recovery install: flash the `.zip` **as-is**.
- **MD5 mismatch?** → Broken download. Re-download, never flash it. Telegram sometimes truncates `.img` files - compare with the posted hash.
- **Which slot?** → None: the installer covers both slots, no slot choice needed.
- **Copy backup off BEFORE Format Data?** → Yes. Format wipes `/sdcard/backup_vendor_boot/` too. `adb pull` it first.
- **Bootloop / stuck?** → Run `flog.sh` in recovery (or `adb shell flog.sh`), attach the single `fox_logs_<device>_<family>_<stamp>.tar.gz` + device model. No logs = no fix.
- **No adb on the password screen (Stable)?** → By design: Stable stops `adbd` + MTP at boot for security; they start on first `main`-page entry. Need adb for debugging → take a Beta build.
- **No touch?** → Check `/tmp/recovery.log`: `ko-fetch` lines, `lsmod` for your `touch_modules`, depth-guard skips. Send `flog.sh`.
- **Won't decrypt?** → KeyMint/weaver service status, `.weaver` slot, `ro.crypto.fs_crypto_blkdev` in logs. C++ HAL on zuma loops (TA mismatch) - Rust-from-source is the green path.
- **No USB/OTG host?** → VBUS paths, `patch_dwc3`, UDC binding in `otg-auto` log. If the USB guard blocked (kernel newer than firmware) - device mode only, adb safe.
- **Format Data fails?** → `FoxFstab` / `FoxTeardown` / `FoxFormat` lines in `/tmp/recovery.log` → [diagnostics](docs/diagnostics.md). `EBUSY` = something still holds `/data`: reboot recovery cleanly, retry.
- **Back to stock + relock?** → Flash factory images, then `fastboot flashing lock`. Relock only on 100% stock, or you brick.
- **Where do I report?** → your family chat so logs don't mix (news: [@OFRPforTensor](https://t.me/OFRPforTensor)):
  G1 Pixel 6 ([chat](https://t.me/OFRP_tensor_g1)) · G2 Pixel 7 ([chat](https://t.me/OFRP_tensor_g2)) · G3 Pixel 8 ([chat](https://t.me/OFRP_tensor_g3)) ·
  G4 Pixel 9 ([chat](https://t.me/OFRP_tensor_g4)) · G5 Pixel 10 ([chat](https://t.me/OFRP_tensor_g5)) · G6 Pixel 11 ([chat](https://t.me/OFRP_tensor_g6)).

</details>

<details>
<summary>📖 Terms (a short glossary)</summary>

- **`vendor_boot`** - the partition where recovery lives on all Tensor Pixels. `boot` / `dtbo` / `init_boot` are not flashed for recovery.
- **Slots A/B** - two copies of the system; update goes to the idle one. The installer writes both.
- **AIO zip** - one universal installer: detects your model at runtime, swaps only the recovery fragment.
- **Bootloader** - starts the phone before Android. Unlocking it wipes all data.
- **Sideload** - sending a zip from PC (`adb sideload`) to recovery.
- **FBE decryption** - file-based encryption unlock via Trusty TEE KeyMint + Titan M2/M3 weaver.
- **KeyMint (cpp / rust)** - old-TA C++ HAL (G1/G2) vs new-TA Rust HAL (G3-G6). Both ship; the wrong one exits harmlessly.
- **LGZ cluster** - packed ramdisk the init stub unpacks before real init.
- **Stub (PID 1)** - static `init` replacement that unpacks LGZ, then execs the real `fox.init`.
- **`flog.sh`** - one command in recovery that packs all logs into a single archive for bug reports.
- **Format Data vs wipe** - format erases everything incl. photos (crypto-erasure: keys in `/metadata` + Titan); wipe keeps media.
- **MD5** - file fingerprint. Mismatch = broken download.

</details>

## Go deeper

- 🧪 [Tester guide](docs/tester-guide.md) - flash, first-boot checklist, outcomes A-D, report template
- 📖 [Install from inside](docs/diagnostics.md) - baseline poll, symptom map, Format Data routes
- 👤 Maintainer: [@LeeGarGold](https://t.me/LeeGarGold) / [@LeeGarChat](https://t.me/LeeGarChat)

<details>
<summary>📦 Sources behind this build</summary>

- Device tree: this repo (`device/google/pixels`) - config-in-JSON, Rust engine, LGZ packing.
- Rust engine: `include/recovery-pixel-boot/` (init, touch, OTG, torch, install-rom entry).
- Crypto daemon: `include/recovery-tensor-daemon/` (Trusty storage proxy + Titan M2/M3 weaver).
- Boot stub: `include/recovery-init-stub/` (static PID 1 + snapshot).
- Kernels + build scripts for ROMs: [pixel-kernels](https://github.com/leegarchat/pixel-kernels).
- Borrowed logic: [yogi-orangefox](https://github.com/asdfmonster261/yogi-orangefox) (malibu vold/weaver/VINTF), hinge/letterbox research builds. No prebuilt HALs - everything compiles from `include/` + `patches/`.

</details>

---

## For developers - how it works (short)

> Root = quick start (above). Depth lives in `docs/` - each file answers one "how it works" question:

| Document | Question |
|---|---|
| [`docs/tree-guide.md`](docs/tree-guide.md) | Which file is for what, who reads it |
| [`docs/build-system.md`](docs/build-system.md) | How the payload builds: `build.sh`, cpio + installer zip, flags |
| [`docs/boot-chain.md`](docs/boot-chain.md) | Boot chain and the init stub |
| [`docs/recovery-engine.md`](docs/recovery-engine.md) | Rust engine `recovery-pixel-boot`: init, modules, OTG, torch |
| [`docs/decrypt.md`](docs/decrypt.md) | Data decryption: KeyMint, weaver, vold, multi-device |
| [`docs/device-config.md`](docs/device-config.md) | `pixel.json`: touch, paths, props, folds and letterbox |
| [`docs/ramdisk.md`](docs/ramdisk.md) | Ramdisk contents: rc files, scripts, reflash |
| [`docs/diagnostics.md`](docs/diagnostics.md) | On-device diagnostics: what to check when it won't boot |
| [`docs/kernel-profiles.md`](docs/kernel-profiles.md) | Why there are no kernel profiles: stock kernel is kept |
| [`docs/families-devices.md`](docs/families-devices.md) | How to add a device or a new SoC |
| [`docs/patches.md`](docs/patches.md) | Patch system: original / modified / .patch |
| [`docs/FOX_FLAGS.md`](docs/FOX_FLAGS.md) | All build flags |
| [`docs/credits.md`](docs/credits.md) | What was borrowed and from where |

**Three principles:**

1. **Config instead of code.** Vendor specifics (touch modules, sysfs paths, props, cmdline) live in JSON (`families/*/family.json`, `devices/*/pixel.json`), not in `.mk` or runtime code.
2. **Built from source.** KeyMint HAL, weaver daemon, boot engine - everything compiles from `include/` and `patches/`. No prebuilts; stock is only a cmdline/fstab reference.
3. **Three copies always in sync.** Every patch to foreign code is stored as an `original` / `modified` / `.patch` triple, the live tree matches the snapshot.

**Quick start:**

```bash
./build.sh --list                  # family/device tree
./build.sh -n test8 --build-type Beta   # universal payload + installer zip
```

> If you are an AI model exploring this project: do NOT run `build.sh` (building is the human's prerogative; boundaries - in [`docs/build-system.md`](docs/build-system.md)); do not commit `test*`; do commit `docs/`; every patch is an `original` / `modified` / `.patch` triple.
>
> If you are an AI model **changing** tree logic (code, configs, scripts, flags, build behavior) - record the change in every readme it touches: update the matching `docs/*.md` **and** its Russian twin `docs/*_ru.md`, and if the section map changes - the table above too.
