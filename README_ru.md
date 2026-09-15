# OrangeFox Device Tree `google/pixels` — руководство (RU)

Мульти-девайсное древо рекавери для Pixel на Tensor SoC (Pixel 6–10:
`gs101`/`gs201`/`zuma`/`zumapro`). Один код — один образ на всё семейство:
девайс определяется в рантайме (`ro.product.device` → `ro.product.name` →
`ro.hardware`, первое с секцией в конфиге; после применения пропсов код
перепроверяется), вендор-специфика подтягивается из JSON-конфига,
собираемого на этапе сборки.

## 1. Структура дерева

```text
device/google/pixels/
├── build.sh                    # точка входа сборки (только руками)
├── vendorsetup.sh              # lunch-хук: резолв семьи, .build_platform.conf
├── device.mk / BoardConfig.mk  # пакеты, оверлеи, Soong
├── twrp_pixels.mk
├── families/                   # SoC-уровень (общее на семейство)
│   ├── common/                 # keymint-пребилды, recovery.wipe, vendor.prop
│   ├── gs101|gs201|zuma|zumapro/
│   │   ├── family.conf         # FAMILY/UFS_ADDR/EARLYCON_ADDR/KEYMINT (для скриптов)
│   │   ├── family.json         # то же + общие пропсы (для мёрджа в конфиг)
│   │   ├── family.mk           # Soong/мake-включения семьи
│   │   ├── fstab/              # пре-рендеренные fstab под vendor_ramdisk
│   │   ├── recovery.fstab      # fstab рекавери
│   │   ├── recovery/root/      # family-оверлей рамдиска (rc-стабы)
│   │   └── etc/                # VINTF-фрагменты (keymint и т.п.)
├── devices/                    # девайс-уровень (только реальные коденеймы)
│   └── <codename>/
│       ├── device.conf         # DEVICE=<codename>, FAMILY=<fam> (для build.sh)
│       ├── pixel.json          # ВЕНДОР-КОНФИГ: модули, разделы, пути, пропсы
│       └── recovery/root/      # per-device оверлей (init.recovery.<dev>.rc)
├── recovery/root/              # общий оверлей: rc, system/bin, vendor/etc
│   ├── init.recovery.pixel_common.rc
│   ├── init.recovery.usb.rc
│   └── system/bin/             # Rust-бины, pixelrunatboot.sh, zip-пейлоады, siw/iw
├── include/                    # исходники Rust-компонентов + пребилды
│   ├── recovery-pixel-boot/    # движок (init/boot/otg-patch/otg-auto/setup-temp/torch)
│   ├── recovery-tensor-daemon/ # storageproxy + weaver (FBE)
│   ├── ramdisk_snapshot/       # снапшот рамдиска для reflash
│   ├── lgz_compress_*          # хост-паковщик / девайс-распаковщик кластера
│   └── otg_host_shim/ / susfs_rename_fix/  # исходники .ko
├── patches/                    # files/-патч-система (см. раздел 9)
├── fox_build_callback.sh       # пост-обработка: мердж конфига + LGZ-пакование
└── custom_bootimg.mk           # сборка vendor_boot (gs101 — патч стока)
```

Принципы:

- `families/` — всё общее для SoC (включая family-оверлей
  `families/<fam>/recovery/`). `devices/` — только коденеймы: конфиг,
  device.conf и per-device оверлей того, что отличается у девайса.
- В образ едут оверлеи **только своей семьи** (`device.mk` фильтрует
  `devices/*` по `device.conf`, мердж конфига — по полю `family`).
  Чужих `init.recovery.*.rc` и секций в образе нет.
- Добавление девайса не требует правок mk: `devices/*` подхватываются
  wildcard'ами (`TARGET_RECOVERY_DEVICE_DIRS`, мердж конфига).

## 2. Сборка

```bash
./build.sh -f shiba        # девайс shiba (Pixel 8) -> семья zuma
./build.sh -f zuma         # сборка сразу на семью
./build.sh -f shiba -l 2   # уровень сжатия кластера 0-3 (дефолт 0)
```

Что происходит:

1. `build.sh -f <X>`: если `families/<X>` существует — это семья; иначе
   читается `devices/<X>/device.conf` (`DEVICE`/`FAMILY`) и выставляется
   `DEVICE_BUILD_FLAG=<семья>`. Списки семей/девайсов строятся динамически
   из каталогов — хардкода нет.
2. `vendorsetup.sh` (lunch) пишет `.build_platform.conf` (семья, UFS-адрес,
   keymint-вариант, `LGZ_LEVEL`/`LGZ_POLICY`): env не переживает ninja
   recipe-shell'ы, поэтому параметры едут файлом.
3. Soong собирает пакеты (`ramdisk_snapshot`, `recovery-tensor-daemon`,
   `recovery-pixel-boot`, fstabs, keymint) + оверлеи:
   `TARGET_RECOVERY_DEVICE_DIRS = корень + devices/* + families/<флаг>`.
4. `--second-call` (`fox_build_callback.sh`, `$TARGET_DIR` = корень рамдиска):
   инжект keymint/VINTF по семье, family-`twrp.flags` поверх дефолта,
   device-оверрайды `<device>.twrp.flags` из `devices/*/twrp.flags`,
   **мердж конфига** (`[PIXELCFG]`), **LGZ-пакование** (`[LGZ]`), генерация
   снапшот-манифеста и списков файлов для `reflash_twrp.sh`.

## 3. LGZ-кластер (сжатие рамдиска)

Файлы рамдиска пакуются в один solid UCOMP02-кластер `/lgz_cluster.lgz`,
который `init` распаковывает до PropertyInit/SELinux/парсинга RC. Зипы
едут внутрь прозрачно (ingest целиком, восстанавливает `lgz decompress`).

Политики (`LGZ_POLICY`, дефолт `dirs`):

- `dirs` — пакуются только каталоги из `LGZ_PACK_DIRS` (скрипты, библиотеки,
  шрифты, прошивочные бинари; `.ko` и `.zip` лежат открыто). Для расширения
  набора: дописать каталог — в логе сборки видно `Entries packed`.
- `all` — всё кроме исключений (максимальное сжатие).

`LGZ_EXCLUDE_LIST` побеждает всегда, в любом режиме. Форматы записей:

| Запись | Смысл |
|---|---|
| `"recovery"` | basename в любом месте |
| `"*.rc"` | по суффиксу |
| `"twres/dir/file.ext"` | точный рамдиск-путь |
| `"twres/subdir/"` | всё дерево под каталогом |

Правило: всё нужное **до** анпака (init-замыкание, `lgz`,
`recovery-pixel-boot`, `recovery`, `*.rc`, манифесты) — только в exclude.

## 4. Конфиг девайсов (`pixel.json`)

Вендор-специфика не живёт в коде — только в JSON. Источники:

- `families/<fam>/family.json`: `family`, `soc_family`, общие `props`.
- `devices/<codename>/pixel.json`: то же + `touch_modules[]`,
  `part_touch`/`part_vendor`, `cs40l26_pm`, пути (`thermal_*`, `torch_*`,
  `vbus_paths[]`, `tcpc_driver`), свои `props`.

Схема секции (плоская, без вложенности кроме `props`):

```json
{
  "family": "zuma",
  "soc_family": "zuma",
  "touch_modules": ["stmvl53l1", "lwis", "...", "fps_touch_handler"],
  "part_touch": "vendor_dlkm",
  "part_vendor": "vendor",
  "cs40l26_pm": "/sys/devices/.../power/control",
  "thermal_zone_types": ["BIG", "CLUSTER2", "..."],
  "thermal_soc_type": "soc_therm",
  "thermal_temp_path": "",
  "torch_i2c_match": "-0063",
  "torch_pinctrl_match": "flash|torch",
  "vbus_paths": ["/sys/class/power_supply/usb/online", "..."],
  "tcpc_driver": "max77759tcpc",
  "props": {"ro.product.model": "Pixel 8", "...": "..."}
}
```

Пути работают в двух режимах: **exact** (непустой `thermal_temp_path` —
используется как есть) и **auto** (пусто — сканирование sysfs в рантайме).
Для `thermal_zone_types`/`vbus_paths`/`torch_*`/`tcpc_driver` отсутствие
ключа = вшитый дефолт (текущие значения).

Мердж (`merge_pixel_config`, `--second-call`, до LGZ): python3 склеивает
все `family.json` + `pixel.json` в `/pixelrunatboot.json` в корне рамдиска
(всегда открыт, в exclude). Пропсы семьи сливаются под пропсы девайса
(девайс побеждает). Невалидный JSON/дубль ключа = громкий фейл сборки.

## 5. Rust-движок `recovery-pixel-boot`

Статический мультиколл-бинанрь (`std` + `liblibc`, `recovery: true`,
`prefer_rlib`). Правило: **Rust делает только syscalls, внешних бинарей
не форкает** — единственная форк-точка это вызов стадий
`pixelrunatboot.sh`.

| Субкоманда | Триггер | Что делает |
|---|---|---|
| `init` | `exec` на `early-init` | резолв девайса → пропсы через стадию → повторный резолв → свап `<device>.twrp.flags` → `twrp.flags`-фикс, magiskboot-распаковка через стадию, `servicemanager.ready` |
| `boot` | `exec` на `on boot` (init ждёт завершения — модули до GUI) | susfs-fix, firmware/модули через стадии + `finit_module`, haptics-PM, magisk-линки + fork-демон, meta-fix через стадию |
| `otg-patch` | сервис `otg_enable` | инжект `otg_host_shim` (скоринг `.ko`), `patch_dwc3=1`, свитч max77759 по I2C |
| `otg-auto` | триггер `patch_dwc3=1` | VBUS-демон host/device (живёт всегда) |
| `setup-temp` | `exec` на `on init` + рефреш из `boot` | симлинк `/dev/thermal_cpu`: exact → `soc_therm` → auto → zone0 |
| `torch on\|off` | `OF_FL_PATH1=cmd:... torch` из GUI | LM3644: дискавери I2C+GPIO(v1 uAPI)+devicetree, нативные ioctl |

Выбор `.ko` (`ko_picker`): ветка ядра × pagesize × Android-поколение из
`uname` (`<mod>_<ver>[_16k].ko` в иерархии `kver/android/pagesize`),
загрузка строго `finit_module(flags=0)` (форс невозможен:
`CONFIG_MODULE_FORCE_LOAD=n`), `EEXIST` = успех.

## 6. `pixelrunatboot.sh` + `siw`/`iw`

Шелл-стадии для работ с внешними бинарями. Протокол: argv + stdout
(результат) + `/tmp/recovery.log` (диагностика), exit 0 = успех.

| Стадия | Инструменты | Что делает |
|---|---|---|
| `props-apply <family> k=v...` | `resetprop`, `setenforce` | ro.*-пропсы (bionic API их блокирует), gs201-флаги |
| `slot-detect` | `bootctl` | суффикс слота в stdout |
| `ko-fetch <part> <sfx> <slot>` | `siw read` → `iw read` | все `*.ko` в `/dev/ko_stage/`; fallback `lptools+mount` |
| `fw-fetch <part> <sfx> <slot>` | `siw`/`iw`, fallback mount | firmware → `/vendor/firmware/` |
| `magiskboot-unpack <zip>` | `unzip`, `busybox` | boot/busybox в `/system/bin` |
| `meta-fix` | `mount` | чистка `/metadata/ota` (с ожиданием блочной ноды) |

`siw`/`iw` — статические arm64-утилиты (1.1M/1.4M): потоковое чтение
разделов (`siw read … | iw read --stdin -c …`) без монтирования и рута.
`lptools_new` оставлен в образе как fallback-путь извлечения.

Стоковый `runatboot.sh` — пустой хук OFox (`twrp.cpp` его дёргает штатно);
точка для аддонов.

## 7. Цепочка загрузки (shiba)

1. Bootloader → ядро + `init_boot` (сток first-stage) + наш `vendor_boot`.
2. First-stage: `IsRecoveryMode()` (`access /system/bin/recovery`, открыт)
   → exec нашего `/system/bin/init`.
3. Наш `init`: `ramdisk_snapshot` (для reflash) → анпак кластера →
   verify (фейл = `reboot,bootloader`) → property/SELinux/RC.
4. `early-init exec` → `recovery-pixel-boot init`.
5. `on init exec` → `setup-temp`; Trusty/keymint/weaver-сервисы.
6. `on boot`: USB-конфиг → `recovery-pixel-boot boot` (синхронно) →
   `start otg_enable` → `patch_dwc3=1` → `start otg_auto`.
7. Сервис `recovery` → GUI → `twrp.cpp` дёргает пустой `runatboot.sh`.

## 8. Добавление нового устройства

1. `devices/<codename>/device.conf` (`DEVICE=`/`FAMILY=`).
2. `devices/<codename>/pixel.json` (секция по схеме из раздела 4;
   `touch_modules` — проверить в стоковом `vendor_dlkm` девайса).
3. `devices/<codename>/recovery/root/init.recovery.<codename>.rc`
   (обычно один `import` общего rc).
4. Опционально `devices/<codename>/twrp.flags` — переопределение поверх
   family-файла (сборщик положит как `<device>.twrp.flags`, рантайм
   подменит после определения девайса).
4. `build.sh -f <codename>` → в логе `[PIXELCFG] merged N devices`,
   `Entries packed`.
5. Прошивка обоих слотов → `dmesg | grep -iE 'LGZ|OFOX|ko_loader'`,
   `/tmp/recovery.log`, `lsmod`, `/dev/input/`.

Новый SoC: + `families/<fam>/` (`family.conf`, `family.json`,
`family.mk`, `fstab/`, `recovery/root/`, `recovery.fstab`,
`twrp.flags` с UFS-путями семьи).

## 9. Патч-система (кратко)

`patches/files/{original,modified}/` + `patches/files/patches/*.patch`
(unified diff). Чек: `apply_patches.py --check` (должно быть всё `[OK]`).
Правило: правится `files/modified/...`, `.patch` регенерируется
`diff -u`, живое дерево приводится к снапшоту — три копии всегда синхронны.

## 10. Диагностика на девайсе (adb, root по умолчанию)

```bash
adb shell 'getprop ro.hardware; uname -r; getconf PAGESIZE'
adb shell 'grep -iE "ko_loader|susfs_fix|otg_patch|pixelrunatboot" /tmp/recovery.log'
adb shell 'ls /dev/ko_stage/*/ | head; cat /proc/modules | grep -cE "touch|goodix"'
adb shell 'dmesg | grep -iE "LGZ|OFOX" | head'
```
