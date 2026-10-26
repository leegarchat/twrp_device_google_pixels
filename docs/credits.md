> [Русская версия](credits_ru.md)

# Borrowings and references

`Guide v3 · 2026-10-26 · what was taken, what was rebuilt`

## yogi-orangefox (Pixel 11 / malibu)

Part of the P11-series logic was spotted and adapted from https://github.com/asdfmonster261/yogi-orangefox (unified tree for malibu: yogi = 11 Pro Fold, cubs/grizzly/kodiak = 11/11 Pro/11 Pro XL).

**What was taken:**

- vold multi-device metadata decrypt: parsing `device=zoned:` / `device=exp:`/`exp_alias:` in libfstab, the `user_devices` field, dm names and key subdirectories by basename, 5-byte `.weaver` slot (BE@1), keySize fallback 0→16, metadata timeout 30→120;
- VINTF KeyMint manifest schema 2.0 (recovery carries libvintf 8.0, stock schema 9.0 crashes registration of **ALL** device HALs);
- Titan M3 weaver protocol (raw structures, hdr `0x000e0000`) - ported into the Rust daemon as a latched fallback, the Titan M2 protobuf path untouched;
- fold cover panel (`graphics_drm.cpp`: DSI selection with the larger `connector_type_id`);
- touch-stack / device-map reference.

**What was NOT taken (deliberately):**

- v5 HAL as a stock prebuilt (we build the Rust HAL from sources), OTG via vendor aocd (our native path), cross-slot reflash (our path - strip dtb + json cmdline), hardcoded cover-panel brightness (the backlight finds itself), health HAL as a prebuilt (we build the generic sysfs `health@2.1-service` from source), `otg_*.sh` scripts (obsolete vs `init.rs`).

<details>
<summary>📖 comet_test_fold (Pixel 9 Pro Fold) + stock firmware</summary>

- From here - hinge detection (EV_SW), `DOF_*` geometry, the letterbox engine (`data.cpp`/`pages.cpp`), vold multi-device portions, the M3 fallback, touch negotiator/offload. The P11 coverage had no hinge/letterbox - here we are ahead of both sources.
- **Stock firmware** is reference only: `vendor_boot` cmdline (byte-for-byte into `kernels`), `vendor_dlkm` (`touch_modules` order), fstab (pre-render). No stock binary snapshots go into the image. Analysis is outside the tree (`roms_extract_pixel/` + `stock_refresh.py` kept by the maintainer).

</details>
