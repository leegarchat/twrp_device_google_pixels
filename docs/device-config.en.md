> [Русская версия](device-config_ru.md)

# Device config (`pixel.json` + `family.json`)

Vendor specifics do not live in code — only in JSON. Code reads the finished
`/pixelrunatboot.json` at the ramdisk root (always open, in LGZ exclusions).

## Sources and merge

- `families/<fam>/family.json`: `family`, `soc_family`, common `props`.
- `devices/<codename>/pixel.json`: same + device hardware (below).
- Merge (`merge_pixel_config` in the callback, `[PIXELCFG]` stage, before LGZ):
  python3 glues all `family.json` + `pixel.json`. Family props merge
  under device props (device wins). Invalid JSON or
  duplicate key = loud build failure.

## Section schema (flat, except `props`)

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
| `touch_modules[]` | `boot` stage | Touch stack load order (verify against stock `vendor_dlkm`) |
| `part_touch` / `part_vendor` / `part_sysdlkm` | `ko-fetch` | Partitions supplying modules and firmware |
| `preload_modules[]` | `boot` stage | Providers before the touch matrix (e.g. `pwrseq-core` for `lwis` on 6.12) |
| `cs40l26_pm` | `boot` stage | Haptics power control |
| `thermal_*` | `setup-temp` | CPU temperature source |
| `torch_*` | `torch` | Flashlight I2C/pinctrl/devicetree matches (LM3644) |
| `vbus_paths[]` | `otg-auto` | VBUS sensors for host/device arbitration |
| `tcpc_driver` | `otg-patch` | Type-C controller (max77759 switch over I2C) |
| `props{}` | `props-apply` | Model props on top of family ones |

Paths work in two modes: **exact** (non-empty path — as is) and
**auto** (empty — runtime sysfs scan). Missing key =
built-in default (current values) — the config can be extended gradually.

## Folds and letterbox (`is_fold` + geometry)

Displays are also in JSON; code knows no resolutions:

```json
{"is_fold": 1, "front_display": {"w": 1080, "h": 2424}, "inner_display": {"w": 2076, "h": 2152}}
```

- `is_fold: 1` — hinge detection and dual geometry; `0`/absent — only
  `front_display` (slab).
- `recovery-pixel-boot init` (on `early-init`, before `DOF_*` is read in
  `data.cpp` and before DRM is opened in minui): scans `/dev/input` for EV_SW
  (`SW_LID` closed = cover, open = inner, no sensor = safe
  cover) and sets `DOF_SCREEN_W/H` of the active canvas plus its
  letterbox flag: `progressive_scale` (default 0 = stock vertical
  stretch) for front/cover/slab, `inner_progressive_scale` (default 1 =
  bars) for the inner canvas. Slab always takes front. Slabs and covers
  carry real panel geometry with 0; inner canvases and the tablet carry
  a 16:9 canvas (see `tools/display_16x9.py`) with 1.
- `wide_theme` (default empty = base `/twres`): wide slabs name their
  pre-generated XML variant (e.g. `"twres_1440"` for 1440-wide panels;
  see `tools/theme_wide.py`), stamped as `ro.recovery.theme` so TWRP's
  dynamic theme pick (gui.cpp) loads matching pages. With a matched
  theme all three scalers agree (~1.0), so text/images/boxes cannot
  drift apart. Tablets and folds stay empty (uniform letterbox path).
- Letterbox engine (`data.cpp` + `pages.cpp`): virtual canvas +
  centering, uniform scale instead of stretching. Without `DOF_SCREEN_W` —
  stock behavior.
- Physical panel selection — separate `graphics_drm.cpp` patch
  (cover DSI with larger `connector_type_id`, otherwise the picture goes to inner
  while touch stays on cover). The parser understands bare numbers (`Val::Num`) and
  `{"w","h"}` objects; old files — default 1080x2400, not fold.
