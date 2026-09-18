> [English version](tree-guide.en.md)

# Путеводитель по дереву `device/google/pixels`

Каждый файл — что это, зачем, кто его читает (сборка / рантайм / разработчик).
Пути — относительно `device/google/pixels/`.

## Точка входа и сборка

| Файл | Назначение | Читает |
|---|---|---|
| `build.sh` | Единственная точка входа сборки. Резолв `-f` (семья или девайс), `-k` (профиль ядра), группировка образов, lunch, `mka`, вызов колбэка. AI-моделям не запускать (см. `build-system_ru.md`). | разработчик |
| `vendorsetup.sh` | Lunch-хук: меню выбора, `TARGET_DEVICE_ALT`/`FOX_TARGET_DEVICES`, все `FOX_*`/`OF_*`/`TW_*` флаги, `OF_FL_PATH1`, пишет `.build_platform.conf` | build-система, `build.sh` |
| `fox_build_callback.sh` | `--second-call` пост-обработка готового рамдиска: мердж конфига, хирургия rc, LGZ-пакование, манифесты для reflash | `build.sh` |
| `gen_kernel_mk.py` | Резолв kernel-профилей из JSON в `.gen_kernel.mk` (`--list/--fingerprint/--generate`) | `build.sh`, разработчик |
| `sync_tree.py` | Умный `repo sync` с сохранением локальных правок + менеджер снапшотов/патчей (`-s/-d/-c/-f`) | разработчик |
| `patches/apply_patches.py` | Применение `patches/files/*.patch` к дереву исходников (`--check` / `--apply`) | `build.sh` |
| `check_keymint.sh` | Ручная проверка keymint на девайсе (md5 HAL, статус сервисов) | разработчик |

## Описание продукта

| Файл | Назначение |
|---|---|
| `twrp_pixels.mk` | Product-определение (`twrp_pixels`, универсальная цель на все Tensor) |
| `device.mk` | Пакеты, оверлеи рамдиска (`TARGET_RECOVERY_DEVICE_DIRS`), фильтры по семье |
| `BoardConfig.mk` | Архитектура, разделы, TWRP/OF-флаги (`TW_FRAMERATE := 120`, яркость, исключения), `-include .gen_kernel.mk` |
| `Android.mk` / `Android.bp` / `AndroidProducts.mk` | Включение в сборку, Soong-модули, список продуктов |
| `custom_bootimg.mk` | Сборка `vendor_boot` (legacy stock-patch режим gs101 superseded, см. `families-devices_ru.md`) |
| `board-info.txt` | Канонический список из 22 девайсов для сборочного забора |

## Данные: семьи и девайсы

| Путь | Назначение |
|---|---|
| `families/<fam>/family.conf` | Shell-факты SoC для скриптов: `FAMILY`, `UFS_ADDR`, `EARLYCON_ADDR`, `USBCTRL` |
| `families/<fam>/family.json` | То же + `keymint` (rust\|cpp), общие `props`, `default_kernel`, `kernels`-профили cmdline |
| `families/<fam>/family.mk` | SoC-фрагмент сборки (без cmdline — он из `.gen_kernel.mk`) |
| `families/<fam>/fstab/` + `recovery.fstab` | Пре-рендеренные fstab под vendor_ramdisk и fstab рекавери. ro-разделы — дублями ext4+erofs (first-stage перебирает дубли одного mountpoint, порядок: ext4, erofs); AVB-флагов (`avb=`, `avb_keys=`) в first-stage нет сознательно — рекавери монтирует без verity; `recovery.fstab` — только ext4, erofs TWRP детектит сам через blkid |
| `families/<fam>/recovery/` | Family-оверлей рамдиска (rc-стабы) |
| `families/<fam>/twrp.flags` | `twrp.flags` семьи (UFS-пути и т.д.) поверх дефолта |
| `families/<fam>/etc/` | VINTF-фрагменты (keymint-манифесты schema 2.0) |
| `families/common/` | Общее без бинарей: `recovery.wipe`, `vendor.prop` |
| `devices/<codename>/device.conf` | `DEVICE=` + `FAMILY=` — привязка для `build.sh` |
| `devices/<codename>/pixel.json` | Вендор-конфиг: модули, разделы, sysfs-пути, пропсы (см. `device-config_ru.md`) |
| `devices/<codename>/recovery/` | Per-device оверлей рамдиска |
| `devices/<codename>/twrp.flags` | Опциональный оверрайд поверх family-файла |

## Исходники (`include/`)

| Путь | Назначение |
|---|---|
| `include/recovery-pixel-boot/` | Rust-движок загрузки: init, модули, OTG, термал, фонарик (см. `recovery-engine_ru.md`) |
| `include/recovery-tensor-daemon/` | Rust-демон: storageproxy + weaver для FBE (см. `decrypt_ru.md`) |
| `include/recovery-init-stub/` | Статический PID 1 (`stub.c` + `snapshot.c`, см. `boot-chain_ru.md`) |
| `include/ramdisk_snapshot/` | Rust-фолбэк снапшота рамдиска (штатно вшит в стаб) |
| `include/lgz_compress_full_x64` / `lgz_compress_lean_arm64` | Хост-паковщик / девайс-распаковщик LGZ-кластера (пребилды-утилиты) |
| `include/otg_host_shim/` + `recovery/root/system/lib64/modules/otg/` | Исходники и готовые `.ko` OTG-шима под все ядра |
| `include/susfs_rename_fix/` + `.ko` | Фикс переименований susfs |
| `fbe_kdf/` (`fox_fbe_kdf.c`) | KDF-плейграунд для FBE-исследований + `recovery/root/system/etc/fox_kdf.conf` (пайплайны) |

## Рамдиск (`recovery/root/`)

| Путь | Назначение |
|---|---|
| `init.recovery.pixel_common.rc` | Общий init: сервисы keymint/weaver, OTG, вызовы движка |
| `init.recovery.usb.rc` | USB-конфигурация (адрес контроллера подставляется под семью) |
| `system/bin/pixelrunatboot.sh` | Шелл-стадии движка (пропсы, слот, модули, firmware) |
| `system/bin/runatboot.sh` | Пустой хук OFox (дёргает `twrp.cpp`; точка для аддонов) |
| `system/bin/reflash_twrp.sh` | Перепрошивка рекавери изнутри рекавери |
| `system/bin/{siw,iw,lptools_new,lpdump}` | Чтение разделов без монтирования, LP-утилиты |
| `system/bin/*.zip` | Пейлоады: Magisk, DFE-NEO, FIXBACKUPKSU, EXPANDPARTITIONS |
| `system/bin/nboot.lz4` | Сжатый компонент загрузки (в LGZ-исключениях) |
| `system/etc/{fox_kdf.conf,task_profiles.json}` | KDF-пайплайны, профили задач |
| `system/etc/vintf/` + `vendor/etc/vintf/` | VINTF-матрицы и манифесты в образе |
| `first_stage-ramdisk-files.txt` | Список файлов first-stage рамдиска |

## Прочее

| Путь | Назначение |
|---|---|
| `docs/` | Эта документация + гайды тестера + `FOX_FLAGS` |
| `vendor-ref/` | Референсные конфиги (`bootctrl`, `conf-<fam>`) — сверка, не сборка |
| `screenshots/` | Скриншоты GUI для постов и гайдов |
| `test_/` | Распакованные образы для анализа (gitignore) |
| `test_ai_handoff.md`, `test_static_init.md` | Исторические заметки сессий (не гайды) |

Зависимости между ключевыми файлами — в документах ниже. Начать:
[`build-system_ru.md`](build-system_ru.md).
