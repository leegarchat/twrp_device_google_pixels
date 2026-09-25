> [Русская версия](README_ru.md)
>
> **Status: public testing.** The full OrangeFox feature set will work in
> the release — right now the **core of a fully working OFox is already
> up**: ADB, MTP, data encryption/decryption, backup/restore, reflash
> recovery, image flashing. What remains is minor per-family debugging:
> vibration, flashlight, OTG and similar device-specific issues.

# OrangeFox for Tensor Pixel — `device/google/pixels`

Universal **AIO** recovery for all Tensor Pixels — from Pixel 6 to
Pixel 11, including folds and the tablet. **One installer zip for every
device**: the exact model is detected at runtime, vendor specifics come
from JSON, HALs are built from source — no vendor prebuilts, stock
kernel is kept.

```bash
./build.sh -f aio --platform-recovery -n test8 -c --build-type Beta
```

This produces `builds/OrangeFox-R12.0-test8-aio.zip` — a single package
that installs working recovery on any supported Pixel (both slots, with
a userdata backup). Per-family images (`-f <family> -k <ver>`) still
exist, but they are a development fallback now: testers only ever see
AIO builds.

| Family | SoC | Devices |
|---|---|---|
| `gs101` | Tensor G1 | oriole (6), raven (6 Pro), bluejay (6a) |
| `gs201` | Tensor G2 | cheetah (7 Pro), panther (7), lynx (7a), felix (Fold), tangorpro (Tablet) |
| `zuma` | Tensor G3 | shiba (8), husky (8 Pro), akita (8a) |
| `zumapro` | Tensor G4 | tokay (9), caiman (9 Pro), komodo (9 Pro XL), tegu (9a), stallion (10a), comet (9 Pro Fold) |
| `laguna` | Tensor G5 | frankel (10), blazer (10 Pro), mustang (10 Pro XL), rango (10 Pro Fold) |
| `malibu` | Tensor G6 | cubs (11), grizzly (11 Pro), kodiak (11 Pro XL), yogi (11 Pro Fold) |

> 100% boot guarantee — only on `shiba` (maintainer's device).
> Everything else goes through community testing:
> [`docs/tester-guide.en.md`](docs/tester-guide.en.md).

## Documentation map

Root is for quick start. Depth lives in `docs/` — each file answers one
"how it works and what it depends on" question:

| Document | Question |
|---|---|
| [`docs/tree-guide.en.md`](docs/tree-guide.en.md) | Which file is for what, who reads it |
| [`docs/build-system.en.md`](docs/build-system.en.md) | How the image builds: `build.sh`, AIO vs family images, flags |
| [`docs/kernel-profiles.en.md`](docs/kernel-profiles.en.md) | Where the kernel cmdline comes from: `family.json` → `.gen_kernel.mk` |
| [`docs/device-config.en.md`](docs/device-config.en.md) | `pixel.json`: touch, paths, props, folds and letterbox |
| [`docs/recovery-engine.en.md`](docs/recovery-engine.en.md) | Rust engine `recovery-pixel-boot`: init, modules, OTG, torch |
| [`docs/boot-chain.en.md`](docs/boot-chain.en.md) | Boot chain and the init stub |
| [`docs/decrypt.en.md`](docs/decrypt.en.md) | Data decryption: keymint, weaver, vold, multi-device |
| [`docs/ramdisk.en.md`](docs/ramdisk.en.md) | Ramdisk contents: rc files, scripts, reflash |
| [`docs/patches.en.md`](docs/patches.en.md) | Patch system: original / modified / .patch |
| [`docs/families-devices.en.md`](docs/families-devices.en.md) | How to add a device or a new SoC |
| [`docs/diagnostics.en.md`](docs/diagnostics.en.md) | On-device diagnostics: what to check when it won't boot |
| [`docs/FOX_FLAGS.md`](docs/FOX_FLAGS.md) | All 70+ OFox build flags |
| [`docs/credits.en.md`](docs/credits.en.md) | What was borrowed and from where |

## Three principles holding it all together

1. **Config instead of code.** Vendor specifics (touch modules, sysfs
   paths, props, kernel cmdline) live in JSON (`families/*/family.json`,
   `devices/*/pixel.json`), not in `.mk` and not in runtime code.
   Adding a device is a new directory, not edits across the tree.
2. **Built from source.** KeyMint HAL, weaver daemon, boot engine —
   everything compiles from `include/` and `patches/`. No prebuilts;
   stock is only a cmdline/fstab reference.
3. **Three copies always in sync.** Every patch to foreign code is stored
   as an `original` / `modified` / `.patch` triple, the live tree matches
   the snapshot. Details — in [`docs/patches.en.md`](docs/patches.en.md).

## Quick start

```bash
./build.sh --list                  # family/device/kernel tree
./build.sh -f aio --platform-recovery -n test8 -c --build-type Beta   # AIO (default route)
./build.sh -f zuma -k 6.12         # per-family fallback (developers only)
./gen_kernel_mk.py --fingerprint zuma 6.12   # who shares an image
```

> If you are an AI model exploring this project, follow the rules:
> do not run `build.sh` (building is the human's prerogative; details
> and the boundaries of your zone — in
> [`docs/build-system.en.md`](docs/build-system.en.md)); do not commit
> `test*`; do commit `docs/`; every patch is an `original` / `modified` /
> `.patch` triple.
>
> If you are an AI model **changing** tree logic (code, configs, scripts,
> flags, build behavior) — record the change in every readme it touches,
> as a rule: update the matching `docs/*.en.md` **and** its Russian twin
> `docs/*_ru.md`, and if the section map changes — the table above too.
