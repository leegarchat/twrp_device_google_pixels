> [English version](device-config.en.md)

# Конфиг девайсов (`pixel.json` + `family.json`)

Вендор-специфика не живёт в коде — только в JSON. Код читает готовый
`/pixelrunatboot.json` в корне рамдиска (всегда открыт, в LGZ-исключениях).

## Источники и мердж

- `families/<fam>/family.json`: `family`, `soc_family`, общие `props`.
- `devices/<codename>/pixel.json`: то же + железо девайса (ниже).
- Мердж (`merge_pixel_config` в колбэке, стадия `[PIXELCFG]`, до LGZ):
  python3 склеивает все `family.json` + `pixel.json`. Пропсы семьи
  сливаются под пропсы девайса (девайс побеждает). Невалидный JSON или
  дубль ключа = громкий фейл сборки.

## Схема секции (плоская, кроме `props`)

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

| Ключ | Кто читает | Смысл |
|---|---|---|
| `touch_modules[]` | `boot`-стадия | Порядок загрузки тач-стека (проверять по стоковому `vendor_dlkm`) |
| `part_touch` / `part_vendor` / `part_sysdlkm` | `ko-fetch` | Разделы-поставщики модулей и firmware |
| `preload_modules[]` | `boot`-стадия | Провайдеры до тач-матрицы (напр. `pwrseq-core` для `lwis` на 6.12) |
| `cs40l26_pm` | `boot`-стадия | Power-control хаптики |
| `thermal_*` | `setup-temp` | Источник температуры CPU |
| `torch_*` | `torch` | Матчи I2C/pinctrl/devicetree фонарика (LM3644) |
| `vbus_paths[]` | `otg-auto` | VBUS-сенсоры для host/device-арбитража |
| `tcpc_driver` | `otg-patch` | Type-C контроллер (свитч max77759 по I2C) |
| `props{}` | `props-apply` | Модельные пропсы поверх семейных |

Пути работают в двух режимах: **exact** (непустой путь — как есть) и
**auto** (пусто — сканирование sysfs в рантайме). Отсутствие ключа =
вшитый дефолт (текущие значения) — конфиг можно дополнять постепенно.

## Фолды и letterbox (`is_fold` + геометрия)

Дисплеи — тоже в JSON, код разрешений не знает:

```json
{"is_fold": 1, "front_display": {"w": 1080, "h": 2424}, "inner_display": {"w": 2076, "h": 2152}}
```

- `is_fold: 1` — hinge-детект и двойная геометрия; `0`/нет — только
  `front_display` (slab).
- `recovery-pixel-boot init` (на `early-init`, до чтения `DOF_*` в
  `data.cpp` и до открытия DRM в minui): сканирует `/dev/input` на EV_SW
  (`SW_LID` закрыт = cover, открыт = inner, нет сенсора = безопасный
  cover) и ставит `DOF_SCREEN_W/H` активного канваса плюс его флаг
  леттербокса: `progressive_scale` (дефолт 0 = стоковый вертикальный
  стретч) для front/cover/slab, `inner_progressive_scale` (дефолт 1 =
  полосы) для внутреннего канваса. Slab всегда берёт front. Слабы и
  каверы несут реальную геометрию панели с 0; внутренние канвасы и
  планшет — 16:9 канвас (см. `tools/display_16x9.py`) с 1.
- Letterbox-движок (`data.cpp` + `pages.cpp`): виртуальный канвас +
  центровка, uniform scale вместо растягивания. Без `DOF_SCREEN_W` —
  стоковое поведение.
- Выбор физической панели — отдельный патч `graphics_drm.cpp`
  (cover DSI с большим `connector_type_id`, иначе картинка на inner,
  а тач на cover). Парсер понимает bare numbers (`Val::Num`) и
  `{"w","h"}`-объекты; старые файлы — дефолт 1080x2400, не фолд.
