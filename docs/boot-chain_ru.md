> [English version](boot-chain.md)

# Цепочка загрузки и init-стаб

`Гайд v3 · 2026-10-26 · от bootloader до GUI`

## Цепочка (пример: shiba)

1. Bootloader → стоковое ядро + `init_boot` (стоковый first-stage) + наш `vendor_boot`.
2. First-stage: стоковая проверка `IsRecoveryMode()` (`access /system/bin/recovery`) → exec нашего `/system/bin/init` - это **статический стаб**, не настоящий init.
3. Стаб: снапшот рамдиска (для reflash) → анпак LGZ-кластера → exec настоящего `fox.init`. Фейл в recovery-режиме = `reboot bootloader`. (`fox.init`, а не `init.real`: без коллизий с цепочками Magisk/KSU.)
4. `early-init exec` → `recovery-pixel-boot init` (резолв девайса, пропсы, hinge-детект).
5. `on init exec` → `setup-temp`; стартуют Trusty / KeyMint / weaver-сервисы.
6. `on boot`: USB-конфиг → `recovery-pixel-boot boot` (синхронно - init ждёт, модули до GUI) → `start otg_enable` → `patch_dwc3=1` → `start otg_auto`.
7. Сервис `recovery` → GUI → `twrp.cpp` дёргает пустой `runatboot.sh`.

<details>
<summary>🧱 Init-стаб (`include/recovery-init-stub/`)</summary>

- Статический C (`static_executable`, без логов) PID 1 вместо `/system/bin/init`; настоящий init едет **внутри LGZ-кластера** как `fox.init` (`init` в exclude-списке, `fox.init` - нет; подмена делается колбэком до паковки и манифестов).
- **Первый вызов** (`fox.init` отсутствует): встроенный снапшот (`snapshot.c`, порт Rust-версии; Rust-бинарь `ramdisk_snapshot` оставлен как фолбэк) → `lgz decompress` → файлы-маркеры (`/lgz_complite`, `/system/etc/lgz_complite`) → exec `fox.init`. Снапшот остаётся видимым в `/ramdisk_snapshot` (+ `/ramdisk_snapshot_manifest.txt`) до ребута - `reflash_twrp.sh` пересобирается из него по требованию.
- **Magisk-канарейка:** в стабе есть 16-байтный литерал, который `magiskinit` хекс-патчит при прошивке (стоковый second-stage путь `/data/magiskinit`). Увидев свою переписанную канарейку, стаб применяет ту же 16→16 подмену к файлу `fox.init` до exec, и стоковый first-stage init чейн-лоадит `magiskinit` ровно как на стоке с Magisk. А если канарейку никто не трогал - `fox.init` остаётся нетронутым.
- **Фолбэк-цепочка, побеждает первое совпадение**: маркеры → `fox.init` → анпак сейчас → хендофф `/init` (Magisk/KSU-хук; защита от петли по readlink - если `/init` это сам стаб, то reboot to bootloader).
- Анпак обязан случиться **до** `selinux_setup`: `fox.init`, sepolicy и пропсы должны лежать на месте до `SetupSelinux` / `PropertyInit`.
- Зачем стаб вообще: файлы, нужные **до** анпака (сам анпакер, `recovery`, `*.rc`, манифесты), не могут ехать внутри кластера. Стаб - минимальный исполняемый мост между first-stage и кластером. Пакет: `PRODUCT_PACKAGES += recovery_init_stub`.

</details>

<details>
<summary>🗜️ LGZ-кластер (сжатие рамдиска)</summary>

- Файлы рамдиска пакуются в один solid UCOMP02-кластер `/lgz_cluster.lgz` (хост-пакеровщик `lgz_compress_full_x64`, девайсный распаковщик `lgz_compress_lean_arm64`), который стаб распаковывает до PropertyInit / SELinux / парсинга RC. Zip'ы едут внутри прозрачно (глотаются целиком, восстанавливаются `lgz decompress`).
- Политики (`LGZ_POLICY`, дефолт `dirs`): `dirs` - только каталоги из `LGZ_PACK_DIRS` (скрипты, библиотеки, шрифты, прошивочные бинари; `.ko` и `.zip` остаются распакованными, расширяется добавлением каталога - `Entries packed` видно в логе). `all` - всё кроме исключений (максимальное сжатие).
- `LGZ_EXCLUDE_LIST` побеждает всегда. Форматы записей: `"recovery"` (basename где угодно), `"*.rc"` (по суффиксу), `"twres/dir/file.ext"` (точный путь в рамдиске), `"twres/subdir/"` (всё дерево).
- Правило: всё, нужное **до** анпака (стаб, `lgz`, `recovery`, `*.rc`, манифесты), - только exclude-списком. Записи линкера/libc в `LGZ_EXCLUDE_LIST` сейчас закомментированы, так что эти библиотеки пакуются в кластер. Мёртвый вес вычищается через `post_remove_ramdisk` в `family.json` (`fox_build_callback.sh`); никакой переменной `LGZ_DROP_LIST` не существует.

</details>
