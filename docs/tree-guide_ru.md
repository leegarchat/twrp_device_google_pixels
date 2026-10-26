> [English version](tree-guide.md)

# Путеводитель по дереву `device/google/pixels`

`Гайд v3 · 2026-10-26 · каждый файл - одна строка`

Каждый файл - что это, зачем, кто его читает (сборка / рантайм / разработчик). Пути - относительно `device/google/pixels/`.

## Точка входа и сборка - старт здесь

| Файл | Назначение | Читает |
|---|---|---|
| `build.sh` | Единственная точка входа сборки (только AIO): фиксированная цель `aio`, lunch, `mka`, извлечение cpio, пакование установщика. AI-моделям не запускать (см. `build-system_ru.md`). | разработчик |
| `vendorsetup.sh` | Lunch-хук: `TARGET_DEVICE_ALT`/`FOX_TARGET_DEVICES` (все девайсы), все `FOX_*`/`OF_*`/`TW_*` флаги, `OF_FL_PATH1`, пишет `.build_platform.conf` | build-система, `build.sh` |
| `include/prebuilt/fox_build_callback.sh` | `--second-call` пост-обработка готового рамдиска: мердж конфига, сборка swap-kit, правки rc, LGZ-пакование, манифесты для reflash | `build.sh` |
| `sync_tree.py` | Умный `repo sync` с сохранением локальных правок + менеджер снапшотов/патчей (`-s/-d/-c/-f`) | разработчик |
| `patches/apply_patches.py` | Применение `patches/files/*.patch` к дереву исходников (`--check` / `--apply`) | `build.sh` |
| `include/prebuilt/check_keymint.sh` | Ручная проверка keymint на девайсе (md5 HAL, статус сервисов) | разработчик |

<details>
<summary>📦 Описание продукта - универсальный таргет</summary>

| Файл | Назначение |
|---|---|
| `twrp_pixels.mk` | Описание продукта (`twrp_pixels`, универсальный таргет под все Tensor) |
| `device.mk` | Пакеты, универсальные оверлеи рамдиска (`TARGET_RECOVERY_DEVICE_DIRS` = все девайсы + семьи), оба KeyMint HAL, без first-stage |
| `BoardConfig.mk` | Архитектура, разделы, TWRP/OF-флаги (`TW_FRAMERATE := 120`, яркость, исключения), фиксированная `aio`-платформа, dummy-cmdline (стоковое ядро сохраняется) |
| `Android.mk` / `Android.bp` / `AndroidProducts.mk` | Включение в сборку, Soong-модули, список продуктов |
| `board-info.txt` | Канонический список 25 девайсов для build-забора |

</details>

<details>
<summary>🗂️ Данные: семьи и девайсы - конфиг, а не код</summary>

| Путь | Назначение |
|---|---|
| `families/aio/family.mk` | Единственный build-фрагмент (размер блока под промежуточный образ; стоковое ядро сохраняется) |
| `families/<fam>/family.conf` | Shell SoC-факты для swap-kit: `FAMILY`, `UFS_ADDR`, `USBCTRL`/`USBBUS` |
| `families/<fam>/family.json` | То же + `keymint` (rust\|cpp\|both), общие `props` |
| `families/<fam>/recovery.fstab` | Посемейный recovery fstab; едет как `recovery.fstab.<fam>` swap-kit (живой файл - aio-заглушка). Остаётся ext4-only - erofs TWRP детектит сам по blkid |
| `families/<fam>/recovery/` | Family-оверлей рамдиска (rc-стабы) |
| `families/<fam>/twrp.flags` | Family `twrp.flags` (пути UFS и т.д.) поверх дефолта |
| `families/<fam>/etc/` | VINTF-фрагменты (KeyMint-манифесты schema 2.0) |
| `families/common/` | Общие файлы без бинарей: `recovery.wipe`, `vendor.prop` |
| `devices/<codename>/device.conf` | `DEVICE=` + `FAMILY=` - привязка для `build.sh` |
| `devices/<codename>/pixel.json` | Вендор-конфиг: модули, разделы, sysfs-пути, пропсы (см. `device-config_ru.md`) |
| `devices/<codename>/recovery/` | Подевайсный оверлей рамдиска |
| `devices/<codename>/twrp.flags` | Опциональный оверрайд поверх family-файла |

</details>

<details>
<summary>🦀 Исходники (`include/`) - работает Rust</summary>

| Путь | Назначение |
|---|---|
| `include/recovery-pixel-boot/` | Rust boot-движок: init, модули, OTG, термал, фонарик (см. `recovery-engine_ru.md`) |
| `include/recovery-tensor-daemon/` | Rust-демон: storageproxy + weaver для FBE (см. `decrypt_ru.md`) |
| `include/recovery-init-stub/` | Статический PID 1 (`stub.c` + `snapshot.c`, см. `boot-chain_ru.md`) |
| `include/ramdisk_snapshot/` | Rust-фолбэк снапшота рамдиска (обычно встроен в стаб) |
| `include/prebuilt/lgz_compress_full_x64` / `lgz_compress_lean_arm64` | Хост-пакеровщик / девайсный распаковщик LGZ-кластера (пребилд-утилиты) |
| `include/source/otg_host_shim/` + `recovery/root/system/lib64/modules/otg/` | Исходники и пребилд `.ko` OTG-шима под все ядра |
| `include/source/susfs_rename_fix/` + `.ko` | susfs rename fix |
| `include/fbe_kdf/` (`fox_fbe_kdf.c`) | KDF-песочница для FBE-исследований + `recovery/root/system/etc/fox_kdf.conf` (пайплайны) |

</details>

<details>
<summary>💿 Рамдиск (`recovery/root/`) - что грузится</summary>

| Путь | Назначение |
|---|---|
| `init.recovery.pixel_common.rc` | Общий init: сервисы KeyMint/weaver, OTG, вызовы движка |
| `init.recovery.usb.rc` | USB-конфигурация (адрес контроллера подставляется по семье) |
| `system/bin/pixelrunatboot.sh` | Shell-стадии движка (пропсы, слот, модули, firmware) |
| `system/bin/runatboot.sh` | Пустой OFox-хук (дёргает `twrp.cpp`; точка расширения для аддонов) |
| `system/bin/reflash_twrp.sh` | Перепрошивка рекавери изнутри рекавери |
| `system/bin/{siw,iw}` | Чтение разделов без монтирования + DM-мэппинг, LP-утилиты |
| `system/bin/*.zip` | Пейлоады: FIXBACKUPKSU, EXPANDPARTITIONS |
| `system/etc/{fox_kdf.conf,task_profiles.json}` | KDF-пайплайны, таск-профили |
| `system/etc/vintf/` + `vendor/etc/vintf/` | VINTF-матрицы и манифесты в образе |
| `first_stage-ramdisk-files.txt` | Список файлов first-stage рамдиска |

</details>

<details>
<summary>🧩 Остальное - прочее</summary>

| Путь | Назначение |
|---|---|
| `docs/` | Эта документация + гайды тестеру + `FOX_FLAGS` |
| `include/vendor-ref/bootctrl/` | Исходники bootctrl HAL 1.2 (собираются из исходников через Soong-неймспейс) |
| `screenshots/` | Скриншоты GUI для постов и гайдов |
| `test_/` | Распакованные образы для анализа (gitignore) |
| `test_ai_handoff.md`, `test_static_init.md` | Исторические заметки сессий (не гайды) |

</details>

Зависимости ключевых файлов раскрыты в документах ниже. Старт: [`build-system_ru.md`](build-system_ru.md).
