> [Русская версия](device-config_ru.md)

# Device config (`pixel.json` + `family.json`)

`Guide v3 · 2026-10-26 · config, not code`

Vendor specifics do not live in code - only in JSON. Code reads the finished `/pixelrunatboot.json` at the ramdisk root (always open, in LGZ exclusions).

## Sources and merge

- `families/<fam>/family.json`: `family`, `soc_family`, common `props`.
- `devices/<codename>/pixel.json`: same + device hardware (below).
- Merge (`merge_pixel_config` in the callback, `[PIXELCFG]` stage, before LGZ): python3 glues all `family.json` + `pixel.json`. Family props merge **under** device props (device wins). Invalid JSON or duplicate key = loud build failure.

<details>
<summary>📐 Section schema (flat, except `props`)</summary>

```json
{
  "family": "zuma",
  "soc_family": "zuma",
  "touch_modules": ["stmvl53l1", "lwis", "...", "fps_touch_handler"],
  "part_touch": "vendor_dlkm",
  "part_vendor": "vendor",
  "part_sysdlkm": "system_dlkm",
  "preload_modules": ["pwrseq-core"],
  "cs40l26_pm": "/sys/devices/.../power/control",
  "thermal_zone_types": ["BIG", "CLUSTER2", "..."],
  "thermal_soc_type": "soc_therm",
  "thermal_temp_path": "",
  "torch_i2c_match": "-0063",
  "torch_pinctrl_match": "flash|torch",
  "vbus_paths": ["/sys/class/power_supply/usb/online", "..."],
  "tcpc_driver": "max77759tcpc",
  "props": {"ro.product.model": "Pixel 8"}
}
```

| Key | Read by | Meaning |
|---|---|---|
| `touch_modules[]` | `boot` stage | Dependency-ordered device modules from stock `vendor_dlkm` (touch, haptics, required support such as Laguna `i2c-dev`) |
| `usb_modules[]` / `part_usb` | `boot` stage, right after touch | Stock USB modules via the same ko-fetch mechanism (11 devices use it: gs101/laguna/malibu sets). Empty (default) = family needs nothing beyond first-stage (zuma, gs201, zumapro: whole Samsung stack autoloads from the ramdisk). `part_usb` default is `vendor_dlkm` |
| `part_touch` / `part_vendor` | `ko-fetch` | Partitions supplying modules and firmware |
| `part_sysdlkm` / `preload_modules` | `boot` stage | Provider modules to insmod BEFORE the touch matrix (e.g. `pwrseq-core` before `lwis` on 6.12). Configured by no device today - the path exists, warns, and stays idle |
| `cs40l26_pm` | `boot` stage | Haptics power control |
| `thermal_*` | `setup-temp` | CPU temperature source (`soc_therm` representative; gs101 panels carry `max_brightness` 2000 - the s6e3hc3 driver reports a bogus max and blanks at 2048) |
| `torch_*` | `torch` | Flashlight I2C/pinctrl/devicetree matches (LM3644) |
| `vbus_paths[]` / `tcpc_driver` | compat only | Kept for JSON compat. The lean per-family USB logic hardcodes its own sensor/driver paths (marked `dead_code`) - editing these keys changes nothing today |
| `max_brightness` | `boot` stage → `DOF_MAX_BRIGHTNESS` → TWRP | Panel brightness ceiling (runtime analogue of static `TW_MAX_BRIGHTNESS`); unset/0 = legacy sysfs `max_brightness` discovery |
| `props{}` | `props-apply` | Model props on top of family ones |

Device identity resolves as `ro.product.device` → `ro.product.name` → `ro.hardware`. The JSON is parsed by a hand-written parser (no serde): flat sections plus `props`, `{"w","h"}` objects or bare numbers for geometry. Invalid JSON or duplicate key = loud build failure.

`status_h` default 130; 150 on caiman, comet, komodo, tokay, tegu, stallion, mustang (stamped as `DOF_STATUS_H` so `data.cpp` drops the compile-time default).

Paths work in two modes: **exact** (non-empty path - as is) and **auto** (empty - runtime sysfs scan). Missing key = built-in default (current values) - the config can be extended gradually.

</details>

<details>
<summary>📱 Folds and letterbox (`is_fold` + geometry)</summary>

- Displays are also in JSON; code knows no resolutions: `{"is_fold": 1, "front_display": {"w": 1080, "h": 2424}, "inner_display": {"w": 2076, "h": 2152}}`.
- `is_fold: 1` - hinge detection and dual geometry; `0`/absent - only `front_display` (slab).
- `recovery-pixel-boot init` (on `early-init`, before TWRP latches geometry and before DRM is opened in minui): scans `/dev/input` for EV_SW (`SW_LID` closed = cover, open = inner, no sensor = safe cover) and sets `DOF_SCREEN_W/H` of the active canvas plus its letterbox flag - `progressive_scale` for front/cover/slab, `inner_progressive_scale` for the inner canvas (both default 0 = stock vertical stretch). Slab always takes front.
- Shipped values are all native panel geometry with 0: slabs/covers carry the real panel size, fold inner canvases carry the native inner panel (e.g. 2076x2152 on rango) with `inner_progressive_scale` 0 + native `inner_theme` (e.g. `twres_2076`), the tangorpro tablet carries 1600x2560 with 0 + `twres_1600` full-bleed. `tools/display_16x9.py` is a dev helper for computing a 16:9 letterboxed canvas if one is ever needed - no shipped device uses scale 1 today.
- `wide_theme` (default empty = base `/twres`): wide slabs name their pre-generated XML variant (e.g. `"twres_1440"` for 1440-wide panels; see `tools/theme_wide.py`), stamped as `DOF_THEME` so TWRP's dynamic theme pick (gui.cpp) loads matching pages. With a matched theme all three scalers agree (~1.0) - text/images/boxes cannot drift apart. The tablet uses `twres_1600` the same way (native full-bleed, no letterbox).
- `inner_theme` (default empty): same for the fold inner canvas, picked by hinge state at boot together with `inner_display`. Closed folds use `wide_theme` (usually empty). Hinge is read **once** at early-init - folding mid-session does not re-pick canvas or theme (pre-existing limit).
- Letterbox engine (`data.cpp` + `pages.cpp`): virtual canvas + centering, uniform scale instead of stretching. Without `DOF_SCREEN_W` - stock behavior.
- Physical panel selection - separate `graphics_drm.cpp` patch (cover DSI with larger `connector_type_id`, otherwise the picture goes to inner while touch stays on cover).

</details>
