> [Русская версия](kernel-profiles_ru.md)

# Kernel profiles (removed)

There are no kernel profiles in this tree anymore: the stock kernel is
always kept and only the universal ramdisk payload is built, so there is
nothing to compose a `VENDOR_CMDLINE` for. `BoardConfig.mk` carries only a
dummy cmdline for the intermediate `vendor_boot` (it donates the platform
fragment the payload is extracted from).

What was removed in the AIO-only cleanup:

- `families/*/family.json` `kernels` / `default_kernel` sections and the
  `_kernels_disabled` per-device overrides in `devices/*/pixel.json`;
- `include/prebuilt/gen_kernel_mk.py` (`--list` / `--fingerprint` /
  `--generate`) and the generated `families/*/.gen_kernel.mk`;
- `build.sh -k/--kernel` / `--force`, the kernel-group loop and per-group
  `*.img` artifacts;
- the `kernel_bootcfg` record in `/pixelrunatboot.json` (reflash rebuilds
  each slot from its own stock image, keeping its header/cmdline/dtb).

This file is kept so existing doc-map links do not break.
