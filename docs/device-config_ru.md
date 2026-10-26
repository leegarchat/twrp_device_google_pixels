> [English version](device-config.md)

# Конфиг девайсов (`pixel.json` + `family.json`)

`Гайд v3 · 2026-10-26 · конфиг, а не код`

Вендор-специфика не живёт в коде - только в JSON. Код читает готовый `/pixelrunatboot.json` в корне рамдиска (всегда открыт, в LGZ-исключениях).

## Источники и мердж

- `families/<fam>/family.json`: `family`, `soc_family`, общие `props`.
- `devices/<codename>/pixel.json`: то же + железо девайса (ниже).
- Мердж (`merge_pixel_config` в колбэке, стадия `[PIXELCFG]`, до LGZ): python3 склеивает все `family.json` + `pixel.json`. Пропсы семьи сливаются **под** пропсы девайса (девайс побеждает). Невалидный JSON или дубль ключа роняют сборку громкой ошибкой.

<details>
<summary>📐 Схема секции (плоская, кроме `props`)</summary>

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

| Ключ | Читает | Значение |
|---|---|---|
| `touch_modules[]` | стадия `boot` | Модули устройства в порядке зависимостей из стокового `vendor_dlkm` (тач, хаптика, нужный саппорт вроде Laguna `i2c-dev`) |
| `usb_modules[]` / `part_usb` | стадия `boot`, сразу после тача | Стоковые USB-модули тем же ko-fetch механизмом (используют 11 девайсов: наборы gs101/laguna/malibu). Пусто (дефолт) = семье ничего не надо сверх first-stage (zuma, gs201, zumapro: весь Samsung-стек самоавтостартует из рамдиска). Дефолт `part_usb` - `vendor_dlkm` |
| `part_touch` / `part_vendor` | `ko-fetch` | Разделы-поставщики модулей и firmware |
| `part_sysdlkm` / `preload_modules` | стадия `boot` | Провайдерные модули до тач-матрицы (напр. `pwrseq-core` перед `lwis` на 6.12). Сегодня не настроены ни у одного девайса - путь существует, варнит и простаивает |
| `cs40l26_pm` | стадия `boot` | Питание хаптики |
| `thermal_*` | `setup-temp` | Источник температуры CPU (`soc_therm` репрезентативный; панели gs101 несут `max_brightness` 2000 - драйвер s6e3hc3 врёт в max и гаснет на 2048) |
| `torch_*` | `torch` | Матчи фонарика I2C/pinctrl/devicetree (LM3644) |
| `vbus_paths[]` / `tcpc_driver` | только совместимость | Оставлены ради JSON-совместимости. Lean-посемейная USB-логика хардкодит свои пути сенсоров/драйвера (помечены `dead_code`) - правка этих ключей сегодня ничего не меняет |
| `max_brightness` | стадия `boot` → `DOF_MAX_BRIGHTNESS` → TWRP | Потолок яркости панели (рантайм-аналог статического `TW_MAX_BRIGHTNESS`); пусто/0 = legacy-поиск `max_brightness` в sysfs |
| `props{}` | `props-apply` | Пропсы модели поверх семейных |

Личность девайса резолвится `ro.product.device` → `ro.product.name` → `ro.hardware`. JSON парсится рукописным парсером (без serde): плоские секции плюс `props`, геометрия объектами `{"w","h"}` или голыми числами. Невалидный JSON или дубль ключа = громкий фейл сборки.

Дефолт `status_h` 130; 150 у caiman, comet, komodo, tokay, tegu, stallion, mustang (штампуется как `DOF_STATUS_H`, чтобы `data.cpp` отбросил компиляционный дефолт).

Пути работают в двух режимах: **exact** (непустой путь - как есть) и **auto** (пусто - рантайм-скан sysfs). Если ключа нет - берётся встроенный дефолт (текущие значения), конфиг растёт постепенно.

</details>

<details>
<summary>📱 Фолды и letterbox (`is_fold` + геометрия)</summary>

- Экраны тоже в JSON; разрешений код не знает: `{"is_fold": 1, "front_display": {"w": 1080, "h": 2424}, "inner_display": {"w": 2076, "h": 2152}}`.
- `is_fold: 1` - hinge-детект и двойная геометрия; `0`/нет - только `front_display` (slab).
- `recovery-pixel-boot init` (на `early-init`, до фиксации геометрии TWRP и до открытия DRM в minui): сканирует `/dev/input` на EV_SW (`SW_LID` closed = cover, open = inner, нет сенсора = safe cover) и выставляет `DOF_SCREEN_W/H` активного канваса + его letterbox-флаг - `progressive_scale` для front/cover/slab, `inner_progressive_scale` для внутреннего канваса (оба дефолт 0 = стоковый вертикальный стретч). Slab всегда берёт front.
- Поставленные значения везде нативные с 0: слэбы/крышки несут реальный размер панели, внутренние канвасы фолдов - нативную inner-панель (напр. 2076x2152 на rango) с `inner_progressive_scale` 0 + нативная `inner_theme` (напр. `twres_2076`), планшет tangorpro - 1600x2560 с 0 + `twres_1600` full-bleed. `tools/display_16x9.py` это dev-хелпер для расчёта 16:9 letterbox-канваса если понадобится - сегодня scale 1 не использует ни один девайс.
- `wide_theme` (дефолт пусто = база `/twres`): широкие слэбы называют свой предгенерированный XML-вариант (напр. `"twres_1440"` для 1440-панелей; см. `tools/theme_wide.py`), штампуется как `DOF_THEME`, и динамический пик темы TWRP (gui.cpp) грузит подходящие страницы. С подходящей темой все три скейлера сходятся (~1.0) - текст/картинки/боксы не разъезжаются. Планшет так же использует `twres_1600` (нативный full-bleed, без letterbox).
- `inner_theme` (дефолт пусто): то же для внутреннего канваса фолда, выбирается по состоянию hinge при загрузке вместе с `inner_display`. Закрытые фолды используют `wide_theme` (обычно пусто). Hinge читается **один раз** на early-init - складывание посреди сессии канвас/тему не перевыбирает (старый лимит).
- Letterbox-движок (`data.cpp` + `pages.cpp`): виртуальный канвас + центрирование, равномерный скейл вместо растягивания. Без `DOF_SCREEN_W` - стоковое поведение.
- Выбор физической панели - отдельный патч `graphics_drm.cpp` (cover DSI с большим `connector_type_id`, иначе картинка уходит на inner, а тач остаётся на cover).

</details>
