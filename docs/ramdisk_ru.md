> [English version](ramdisk.en.md)
# Содержимое рамдиска (`recovery/root/`)

В образ едут оверлеи **только своей семьи** (`device.mk` фильтрует
`devices/*` по `device.conf`, мердж конфига — по полю `family`). Чужих
`init.recovery.*.rc` и секций в образе нет.

## Init-файлы (корень рамдиска)

| Файл | Роль |
|---|---|
| `init.recovery.pixel_common.rc` | Общий init: сервисы keymint/weaver, OTG (`otg_enable`/`otg_auto`), `exec` вызовы движка на `early-init`/`on init`/`on boot`. Объект хирургии колбэка: подстановка USB-адреса, гашение чужого keymint |
| `init.recovery.usb.rc` | USB-конфигурация (DWC3-платформа; `11210000` для gs201/zuma/zumapro, замена через `USBCTRL` для laguna/malibu) |
| `first_stage-ramdisk-files.txt` | Список файлов first-stage рамдиска |

## `system/bin/` — исполняемое

| Файл | Роль |
|---|---|
| `recovery-pixel-boot` | Rust-движок (см. `recovery-engine_ru.md`) |
| `recovery-tensor-daemon` | Weaver/storageproxy-демон (см. `decrypt_ru.md`) |
| `recovery_init_stub` → `init` | Init-стаб (свап в колбэке, см. `boot-chain_ru.md`) |
| `pixelrunatboot.sh` | Шелл-стадии движка |
| `runatboot.sh` | Пустой хук OFox |
| `reflash_twrp.sh` | Перепрошивка изнутри рекавери (ниже) |
| `siw`, `iw` | Чтение разделов без монтирования + DM-маппинг, LP-инструменты |
| `nboot.lz4` | Сжатый компонент загрузки (в LGZ-исключениях) |
| `Magisk-*.zip`, `FIXBACKUPKSU.zip`, `EXPANDPARTITIONS.zip` | Пейлоады для установки из GUI |

## `system/etc/` и `vendor/etc/`

| Путь | Роль |
|---|---|
| `system/etc/fox_kdf.conf` | KDF-пайплайны (см. `decrypt_ru.md`) |
| `system/etc/task_profiles.json` | Профили задач |
| `system/etc/vintf/` + `vendor/etc/vintf/` | VINTF-матрицы совместимости и HAL-манифесты (keymint schema 2.0) |
| `vendor/etc/ueventd.rc` | Ueventd-правила вендора |

## `system/lib64/modules/`

- `otg/` — `otg_host_shim*.ko` под каждую ветку ядра × pagesize ×
  поколение Android (выбор — `ko_picker` движка).
- `susfs_rename_fix.ko` — фикс переименований susfs.

## Reflash изнутри (`reflash_twrp.sh`)

Перепрошивка рекавери без ПК: берёт `kernel_bootcfg`-запись
(`FOX_KERNEL_VER` штампуется колбэком из `.gen_kernel.mk`) и списки
файлов из снапшот-манифеста колбэка. Образ собирается **без DTB**
(`DTB_SZ 0`, vendor_boot dtb-free) — strip обязателен, иначе
перезапись затрёт соседние фрагменты. Штамп `unpack -h`
(header/bootconfig) пишется single-record.
