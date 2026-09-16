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
├── build.sh                    # точка входа сборки (только руками; -f/-k/--force)
├── gen_kernel_mk.py            # резолв kernel-профилей -> .gen_kernel.mk (см. раздел 11)
├── vendorsetup.sh              # lunch-хук: резолв семьи, .build_platform.conf
├── device.mk / BoardConfig.mk  # пакеты, оверлеи, Soong
├── twrp_pixels.mk
├── families/                   # SoC-уровень (общее на семейство)
│   ├── common/                 # recovery.wipe, vendor.prop (без бинарей)
│   ├── gs101|gs201|zuma|zumapro|malibu/
│   │   ├── family.conf         # FAMILY/UFS_ADDR/EARLYCON_ADDR (для скриптов)
│   │   ├── family.json         # то же + keymint rust|cpp + общие пропсы + kernels-профили (для мёрджа в конфиг)
│   │   ├── family.mk           # SoC-фрагмент (без cmdline: он едет из .gen_kernel.mk)
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
├── include/                    # исходники компонентов + пребилды
│   ├── recovery-pixel-boot/    # движок (init/boot/otg-patch/otg-auto/setup-temp/torch)
│   ├── recovery-init-stub/     # static PID 1 (stub.c + snapshot.c, см. раздел 12)
│   ├── recovery-tensor-daemon/ # storageproxy + weaver (FBE)
│   ├── ramdisk_snapshot/       # Rust-фолбэк снапшота (штатно вшит в стаб)
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
./build.sh -f zuma -k 6.12 # kernel-профиль 6.12 (см. раздел 11)
./build.sh -f zuma -k 6.1 --force  # то же, молча (для скриптов/CI)
```

Без `-k` — интерактивный выбор профиля; без `-k` + `--force` — ошибка
со списком доступных версий (молчаливого дефолта в скриптах нет).

Что происходит:

1. `build.sh -f <X>`: если `families/<X>` существует — это семья; иначе
   читается `devices/<X>/device.conf` (`DEVICE`/`FAMILY`) и выставляется
   `DEVICE_BUILD_FLAG=<семья>`. Списки семей/девайсов строятся динамически
   из каталогов — хардкода нет.
2. `vendorsetup.sh` (lunch) пишет `.build_platform.conf` (семья, UFS-адрес,
   keymint-вариант, `LGZ_LEVEL`/`LGZ_POLICY`): env не переживает ninja
   recipe-shell'ы, поэтому параметры едут файлом.
3. Soong собирает пакеты (`recovery_init_stub`, `recovery-tensor-daemon`,
    `recovery-pixel-boot`, fstabs, keymint) + оверлеи:
    `TARGET_RECOVERY_DEVICE_DIRS = корень + devices/* + families/<флаг>`.
    KeyMint HAL всегда из исходников (без пребилдов): тип выбирается полем
    `keymint` (`rust`|`cpp`) из `families/<семья>/family.json` — `build.sh`
    кладёт его в `FOX_KEYMINT_TYPE` для `device.mk` и в таргеты сборки,
    `vendorsetup.sh` дублирует в `.build_platform.conf` для колбэка.
    `VENDOR_CMDLINE` на этом этапе уже переопределён из `.gen_kernel.mk`
    (сгенерирован до lunch — `dumpvars` парсит BoardConfig во время lunch).
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

Правило: всё нужное **до** анпака (статический стаб, `lgz`,
`recovery`, `*.rc`, манифесты) — только в exclude. Init-замыкание
(linker/libc/...) с появлением стаба едет **внутри** кластера.
`LGZ_DROP_LIST` в колбэке удаляет мёртвый вес из стейджинга до паковки
(проверка `readelf NEEDED` + `strings` на dlopen обязательна; CJK-шрифт
и installer-zip'ы дропать только для тестовых сборок).

## 4. Конфиг девайсов (`pixel.json`)

Вендор-специфика не живёт в коде — только в JSON. Источники:

- `families/<fam>/family.json`: `family`, `soc_family`, общие `props`.
- `devices/<codename>/pixel.json`: то же + `touch_modules[]`,
  `part_touch`/`part_vendor`, `part_sysdlkm` + `preload_modules[]`
  (провайдеры до touch-матрицы, напр. `pwrseq-core` для `lwis` на 6.12),
  `cs40l26_pm`, пути (`thermal_*`, `torch_*`,
  `vbus_paths[]`, `tcpc_driver`), свои `props`.
- `families/<fam>/family.json`: + `default_kernel`, `kernels` (раздел 11).

Схема секции (плоская, без вложенности кроме `props`):

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

## 4.1. Фолды и letterbox (is_fold + display-геометрия)

Дисплеи тоже переехали в JSON (единый источник, код не знает разрешений):

```json
{
  "is_fold": 1,
  "front_display": {"w": 1080, "h": 2424},
  "inner_display": {"w": 2076, "h": 2152}
}
```

- `is_fold: 1` — фолд: применяются hinge-детект и двойная геометрия.
  `0`/отсутствует — только `front_display` (база для slab).
- `recovery-pixel-boot init` (`on early-init`, до чтения DOF_ в data.cpp
  и до открытия DRM в minui): на фолде сканирует `/dev/input` на EV_SW
  (`SW_LID` закрыт = cover, `SW_TABLET_MODE`/`lid` открыт = inner, нет
  сенсора = безопасный дефолт cover) и ставит `DOF_SCREEN_W/H` активного
  канваса + `DOF_PROGRESSIVE_SCALE=1`. Slab всегда берёт front.
- Letterbox-движок (`data.cpp` + `pages.cpp` в снапшотах): виртуальный
  канвас + центровка, uniform scale вместо растягивания. Без
  `DOF_SCREEN_W`/progressive — стоковое поведение без изменений.
- Парсер конфига понимает bare numbers (`Val::Num`) и вложенные
  `{"w","h"}`-объекты; старые файлы без ключей парсятся в дефолты
  (1080x2400, не фолд).

## 5. Rust-движок `recovery-pixel-boot`

Статический мультиколл-бинанрь (`std` + `liblibc`, `recovery: true`,
`prefer_rlib`). Правило: **Rust делает только syscalls, внешних бинарей
не форкает** — единственная форк-точка это вызов стадий
`pixelrunatboot.sh`.

| Субкоманда | Триггер | Что делает |
|---|---|---|
| `init` | `exec` на `early-init` | резолв девайса → пропсы через стадию → повторный резолв → свап `<device>.twrp.flags` → `twrp.flags`-фикс, magiskboot-распаковка через стадию, `servicemanager.ready` |
| `boot` | `exec` на `on boot` (init ждёт завершения — модули до GUI) | preload (`part_sysdlkm`/`preload_modules`, best-effort) → susfs-fix, firmware/модули через стадии + `finit_module`, haptics-PM, magisk-линки + fork-демон, meta-fix через стадию |
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
| `ko-fetch <part> <sfx> <slot>` | `siw read` → `iw read` | все `*.ko` в `/dev/ko_stage/`; fallback `lptools+mount`. Depth-guard: только `lib/modules/*.ko` — подкаталоги pagesize (`16k-mode/`) пропускаются (их близнецы с чужими CRC затеняли плоские модули и ломали тач/хаптику на 6.12) |
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
   → exec нашего `/system/bin/init` (статический стаб, раздел 12).
3. Стаб: `ramdisk_snapshot` (для reflash) → анпак кластера →
   exec настоящего `init.real`. Фейл = `reboot,bootloader`.
   Legacy-путь в `SecondStageMain` пропускается (handoff по `init.real`).
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

Новый SoC: + `families/<fam>/` (`family.conf`, `family.json`
(+ `kernels`/`default_kernel`, раздел 11), `family.mk` (без cmdline),
`fstab/`, `recovery/root/`, `recovery.fstab`,
`twrp.flags` с UFS-путями семьи).

## 11. Kernel-профили (`kernels` в JSON + `-k`)

`VENDOR_CMDLINE`/`BOARD_BOOTCONFIG` живут не в `.mk`, а в
`families/<fam>/family.json` (`kernels`, ключ = версия: `6.1`, `6.12`).
Схема профиля: `cmdline` — типизированный конверт
(`str` = одна строка, `arr` = `["k=v", …]`, `list` = `[{k: v}, …]`),
`bootconfig_append` — список, `prebuilt`/`ko` — резерв под следующие шаги.
Оверрайд девайса (`devices/<dev>/pixel.json` → `kernels[VER]`) заменяет
семейный профиль **целиком**. Девайсы с одинаковым эффективным профилем
(сравнение пофлагово, порядок не важен) собираются в **один** образ
(`zuma.img`); разошедшийся — в свой (`zuma_husky.img`, подгруппа —
`zuma_husky-akita.img`).

Механика: `build.sh -k` → `gen_kernel_mk.py --fingerprint` (группы) →
`.gen_kernel.mk` в `families/<fam>/` (gitignore; `VENDOR_CMDLINE`,
`BOARD_BOOTCONFIG += …`, `FOX_KERNEL_VER`) → `-include` в конце
`BoardConfig.mk`. Пре-генерация — до lunch (`dumpvars` парсит BoardConfig
во время lunch); между группами — обязательная чистка `PRODUCT_OUT`;
после сборки файл удаляется. Пустой `VENDOR_CMDLINE` = громкий
`$(error)` вместо незагружаемого образа.

```bash
./gen_kernel_mk.py --list zuma            # версии + (default: …)
./gen_kernel_mk.py --fingerprint zuma 6.12  # группы: device/hash/source
```

## 12. Init-стаб (`recovery-init-stub`)

Статический (C, `static_executable`, без логов) PID 1 на месте
`/system/bin/init`; настоящий init едет внутри кластера как `init.real`
(`init` в exclude, `init.real` — нет; свап в колбэке до паковки и
манифестов). Первое invocation (`init.real` отсутствует): встроенный
снапшот (`snapshot.c`, порт Rust-версии; Rust-бинарь оставлен фолбэком) →
`lgz decompress` → exec `init.real`; неуспех в recovery-режиме —
`reboot bootloader`. Последующие (`second_stage`, `init.real` на месте) —
мгновенный passthrough с сохранением argv/env. Распаковка на
`selinux_setup` обязательна: `init.real` и sepolicy/пропсы должны быть
на месте до `SetupSelinux`/`PropertyInit`.

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

## 13. Pixel 11 (malibu): заимствования из yogi-orangefox

Часть логики P11-серии подсмотрена и адаптирована из
https://github.com/asdfmonster261/yogi-orangefox ( unified-дерево под
malibu: yogi = 11 Pro Fold, cubs/grizzly/kodiak = 11/11 Pro/11 Pro XL).
Что взято:
- vold multi-device metadata decrypt: парсинг `device=zoned:` /
  `device=exp:`/`exp_alias:` в libfstab, поле `user_devices`, dm-имена и
  key-подкаталоги по basename, 5-байтный `.weaver`-слот (BE@1),
  фолбэк keySize 0→16, таймаут метадаты 30→120;
- VINTF keymint-манифест schema 2.0 (recovery везёт libvintf 8.0, schema 9.0
  из стока роняет регистрацию ВСЕХ device-HAL);
- протокол Titan M3 weaver (raw-структуры, hdr `0x000e0000`) — перенесён
  в наш Rust-демон как фолбэк с лэтчем, protobuf-путь Titan M не тронут;
- референс тач-стека/карты девайсов (sec_touch через GTI на фолде,
  focal+syna на candybar, dep-порядок).
Что НЕ взято (сознательно): v5-HAL пребилдом из стока (собираем Rust-HAL
из исходников), OTG через vendor aocd (наш native-путь встал сам),
попереслотный reflash (наш путь — strip dtb + json-cmdline),
семейный хардкод яркости cover-панели (на grizzly подсветка находится
сама на `panel0-backlight`).
