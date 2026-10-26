> [English version](ramdisk.md)

# Содержимое рамдиска (`recovery/root/`)

`Гайд v3 · 2026-10-26 · что едет внутри образа`

Оверлеи **всех девайсов + всех семей** едут в один универсальный cpio (`TARGET_RECOVERY_DEVICE_DIRS` = корень + все `devices/*` + все `families/*` в `device.mk`). Правильные семейные файлы выбираются установщиком + boot-движком при установке/загрузке.

<details>
<summary>📜 Init-файлы (корень рамдиска)</summary>

| Файл | Роль |
|---|---|
| `init.recovery.pixel_common.rc` | Общий init: сервисы KeyMint/weaver, OTG (`otg_enable`/`otg_auto`), `exec` вызовы движка на `early-init`/`on init`/`on boot`. Место правок колбэка: подстановка USB-адреса, отключение чужого KeyMint |
| `init.recovery.usb.rc` | USB-конфигурация (DWC3-платформа; `11210000` для gs201/zuma/zumapro, замена через USBCTRL для laguna/malibu) |
| `first_stage-ramdisk-files.txt` | Список файлов first-stage рамдиска |

</details>

<details>
<summary>⚙️ `system/bin/` - исполняемое</summary>

| Файл | Роль |
|---|---|
| `recovery-pixel-boot` | Rust-движок (см. `recovery-engine_ru.md`) |
| `recovery-tensor-daemon` | Демон weaver/storageproxy (см. `decrypt_ru.md`) |
| `recovery_init_stub` → `init` | Init-стаб (подменяется колбэком, см. `boot-chain_ru.md`) |
| `pixelrunatboot.sh` | Shell-стадии движка (пропсы, слот, модули, firmware) |
| `runatboot.sh` | Пустой OFox-хук (дёргает `twrp.cpp`; точка расширения для аддонов) |
| `reflash_twrp.sh` | Перепрошивка изнутри рекавери (ниже; тул: `/system/bin/bootsmasher-install`, стейджится колбэком сборки из `include/prebuilt/bootsmasher-install-arm64`) |
| `flog.sh` | Сборщик логов одним заходом: `fox_logs_<device>_<family>_<stamp>.tar.gz` → `/data/media/0/fox_logs/` (или `/tmp`, + подсказка `adb pull`). Пакует recovery/weaver/dmesg/logcat, backlight + снапшот батареи/health, pstore, логи установщика |
| `otg_test.sh` | Полевой тест OTG T1-T8 (диск, раздел, демон, линк `/dev/block/otg-usb`, match uevent, тома, mount, dedup) → `otg_test_<device>_<family>_<stamp>.tar.gz` рядом с архивами flog. Гонять на девайсе с подключённым диском и отключённым ПК |
| `siw`, `iw` | Чтение разделов без монтирования + DM-мэппинг, LP-утилиты |
| `lptools_new` | DM-мэппинг через системный fs_mgr стек (`--map`/`--unmap`), стейджинг-фолбэк |
| `fox-hsinstall` | Движок скоростной установки ROM (статический Rust-пребилд) |
| `FIXBACKUPKSU.zip`, `EXPANDPARTITIONS.zip` | Пейлоады для установки из GUI |

</details>

<details>
<summary>📁 `system/etc/`, `vendor/etc/`, модули</summary>

| Путь | Роль |
|---|---|
| `system/etc/fox_kdf.conf` | KDF-пайплайны (см. `decrypt_ru.md`) |
| `system/etc/task_profiles.json` | Таск-профили |
| `system/etc/vintf/` + `vendor/etc/vintf/` | VINTF-матрицы совместимости и HAL-манифесты (KeyMint schema 2.0) |
| `vendor/etc/ueventd.rc` | Вендорские ueventd-правила |
| `system/lib64/modules/otg/` | `otg_host_shim*.ko` / `aoc_vote_shim*.ko` под каждую ветку ядра × pagesize × поколение Android (выбирает `ko_picker` движка) |
| `system/lib64/modules/probe/` | Канареечный набор `empty_checker_module` (доказательство монолита / инклюзии, та же раскладка kver/android/pagesize) |
| `system/lib64/modules/touch/` | Подевайсные тач-архивы (`<device>.lgz`, пушит установщик через `--inject-lgz`, распаковывает стадия `touch-unpack`) |
| `susfs_fix` | susfs rename fix |

</details>

<details>
<summary>🔄 Перепрошивка изнутри (`reflash_twrp.sh`)</summary>

- Перепрошивка рекавери без ПК (`bootsmasher-install install --file`): снимаются снапшоты **обоих** слотов `vendor_boot`, каждый образ пересобирается из **своего же** стокового образа (умная замена: first_stage, header, cmdline и dtb берутся из самого слота, ничего не штампуется). Отсутствующий `/ramdisk_snapshot` пересобирается по требованию из `/ramdisk_snapshot_manifest.txt` (один проход cpio, без хелпер-бинаря).
- Recovery-only cpio-пейлоад строится из `/ramdisk_snapshot` (пути first_stage исключены - каждый слот даёт свои; живой `/<device>.lgz` дописывается в манифест, чтобы тач-пакет выжил). Оба слота шьются только при соблюдении политики MIN_FREE_MB, затем проверяются обратным сравнением.
- Бэкапы стоковых образов - в `/sdcard/backup_vendor_boot/`, когда userdata доступна на запись.

</details>
