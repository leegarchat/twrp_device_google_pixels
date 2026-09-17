> [Русская версия](families-devices_ru.md)
# Adding a device or a new SoC

Adding a device requires no `.mk` edits: `devices/*` are picked up
via wildcards (`TARGET_RECOVERY_DEVICE_DIRS`, config merge).

## New device (5 steps)

1. `devices/<codename>/device.conf`:
   `DEVICE=<codename>`, `FAMILY=<fam>`.
2. `devices/<codename>/pixel.json` — section per the schema in
   `device-config.en.md`. Verify `touch_modules` against the stock
   `vendor_dlkm` of the device; for a fold add `is_fold` + geometries.
3. `devices/<codename>/recovery/root/init.recovery.<codename>.rc`
   (usually a single `import` of the shared rc).
4. Optionally `devices/<codename>/twrp.flags` — on top of the family file
   (the builder places it as `<device>.twrp.flags`, the runtime swaps it in after
   device resolution).
5. `build.sh -f <codename>` → in the log `[PIXELCFG] merged N devices`,
   `Entries packed`. Flash both slots → checks from
   `diagnostics.en.md`.

## New SoC (family `families/<fam>/`)

| File | Contents |
|---|---|
| `family.conf` | `FAMILY`, `UFS_ADDR`, `EARLYCON_ADDR`, `USBCTRL` (if not `11210000.dwc3`) |
| `family.json` | Same + `keymint` (rust\|cpp), shared `props`, `default_kernel`, `kernels` profiles (see `kernel-profiles.en.md`) |
| `family.mk` | SoC build fragment (no cmdline!) |
| `fstab/` + `recovery.fstab` | Pre-rendered fstabs |
| `recovery/` | Family ramdisk overlay (rc stubs) |
| `twrp.flags` | Family flags (UFS paths, etc.) |
| `etc/` | VINTF fragments (keymint manifest schema 2.0) |

Next: stock `vendor_boot` of the device → capture the cmdline byte-for-byte →
profile in `kernels` → `board-info.txt` → test per `tester-guide.en.md`.

## Family matrix (facts for configs)

| Family | UFS | DWC3 USB | KeyMint | Kernels |
|---|---|---|---|---|
| gs201 | `14700000` | `11210000.dwc3` | cpp | 6.1, 6.12 |
| zuma | `13200000` | `11210000.dwc3` | rust | 6.1, 6.12 |
| zumapro | `13200000` | `11210000.dwc3` | rust | 6.1, 6.12 |
| laguna | `3c400000` | `c400000.dwc3` | rust | 6.12 |
| malibu | `3c2d0000` | `a210000.dwc3` | rust | 6.12 |
| gs101 | — | — | cpp | WIP |

Devices are grouped into a single image per family (`kernels` overrides
disabled via `_kernels_disabled`; to restore — rename the key).
