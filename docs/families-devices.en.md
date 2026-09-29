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
5. `build.sh -n <tag>` → in the log `[PIXELCFG] merged N devices`,
   `Entries packed`. Flash both slots via the installer → checks from
   `diagnostics.en.md`.

## New SoC (family `families/<fam>/`)

| File | Contents |
|---|---|
| `family.conf` | `FAMILY`, `UFS_ADDR`, `USBCTRL` (if not `11210000.dwc3`), `USBBUS` (if the controller lives under `simple_usb_bus`) |
| `family.json` | Same + `keymint` (rust\|cpp), shared `props` |
| `recovery.fstab` + `recovery.wipe` + `twrp.flags` | Swap-kit files: ship as `*.<fam>`, the installer selects post-unpack |
| `recovery/` | Family ramdisk overlay (rc stubs) |
| `etc/` | VINTF fragments (keymint manifest schema 2.0) |

Next: stock `vendor_boot` of the device → verify the UFS/USB addresses
against `twrp.flags` → `board-info.txt` → test per `tester-guide.en.md`.
No kernel profiles (stock kernel is kept — see `kernel-profiles.en.md`),
no `family.mk` (only `families/aio` carries a build fragment).

## Family matrix (facts for configs)

| Family | UFS | DWC3 USB | KeyMint |
|---|---|---|---|
| gs201 | `14700000` | `11210000.dwc3` | cpp |
| zuma | `13200000` | `11210000.dwc3` | rust |
| zumapro | `13200000` | `11210000.dwc3` | rust |
| laguna | `3c400000` | `c400000.dwc3` | rust |
| malibu | `3c2d0000` | `a210000.dwc3` | rust |
| gs101 | `14700000` | `11110000.dwc3` | cpp |

Every device ships in the single universal payload; family files are
selected at install/boot time (swap kit + stub + engine).

## gs101 (Tensor G1, Pixel 6 series) — hardware notes

gs101 has no `vendor_kernel_boot` partition: stock `vendor_boot` carries
platform + dlkm + dtb fragments. The universal payload is a platform
fragment (first-stage + recovery merged), flashed as fragment surgery —
`fastboot flash vendor_boot: <ramdisk>` (empty name = the platform
fragment; NOT `:default` — that collapses the whole section into one
entry and kills the stock dlkm). The installer fetches the on-device
`vendor_boot` itself, swaps only the platform entry and flashes it
back: stock dlkm+dtb survive, first-stage keeps autoloading the stock
modules from dlkm.

Manual flashing (what the installer automates):

```bash
fastboot flash vendor_boot: OrangeFox-R12.0-test_x-aio.ramdisk.lz4
fastboot reboot recovery
```
