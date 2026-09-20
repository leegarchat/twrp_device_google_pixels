> [Русская версия](kernel-profiles_ru.md)

# Kernel profiles (`kernels` in JSON + `-k`)

`VENDOR_CMDLINE` and `BOARD_BOOTCONFIG` live not in `.mk` but in data:
`families/<fam>/family.json`, `kernels` section (key — version: `6.1`, `6.12`).
This way cmdline is versioned together with stock and verified byte-for-byte.

## Profile schema

```json
"kernels": {
  "6.12": {
    "cmdline": ["k=v", "..."],
    "bootconfig_append": ["..."]
  }
}
```

The `cmdline` envelope is typed: `str` — single string, `arr` — `["k=v", …]`,
`list` — `[{k: v}, …]`. `bootconfig_append` — list of bootconfig additions.
`prebuilt`/`ko` — reserved for next steps.

A device override (`devices/<dev>/pixel.json` → `kernels[VER]`) replaces the
family profile **entirely**; it does not patch it flag-by-flag. All
overrides are currently disabled by renaming the key to `_kernels_disabled`
(content kept in place for rollback) — see `build-system.en.md`.

## Resolution mechanics

```
build.sh -k VER
  → gen_kernel_mk.py --fingerprint FAM VER   # groups device/hash/source
  → families/<fam>/.gen_kernel.mk            # gitignore, before lunch!
  → -include at the end of BoardConfig.mk    # VENDOR_CMDLINE, BOARD_BOOTCONFIG, FOX_KERNEL_VER
  → ... build ...
  → file is deleted after the build
```

- Pre-generation must run before lunch: `dumpvars` parses BoardConfig
  right during lunch; later the cmdline will no longer be picked up.
- Between kernel groups — `PRODUCT_OUT` cleanup, otherwise artifacts get mixed.
- Empty `VENDOR_CMDLINE` = `$(error)` in BoardConfig: better a loud
  failure than an unbootable image.
- `kernel_bootcfg` in the image is assembled from `.gen_kernel.mk`
  (`FOX_KERNEL_VER` is stamped into the reflash record) — image cmdline and
  reflash always come from a single source.

## Commands

```bash
./include/prebuilt/gen_kernel_mk.py --list zuma                  # versions + (default: …)
./include/prebuilt/gen_kernel_mk.py --fingerprint zuma 6.12      # groups: device/hash/source
./include/prebuilt/gen_kernel_mk.py --generate zuma shiba 6.12 out.mk  # profile to file (compare with stock)
```

`default_kernel` in `family.json` — version without `-k` in interactive mode.
Reference cmdlines are taken from stock `vendor_boot` (Beta5: 6.1 kernels,
Beta4: 6.12; Beta5 cmdline = Beta4 + `binder.impl=rust`).
