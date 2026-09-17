> [English version](boot-chain.en.md)
# Цепочка загрузки и init-стаб

## Цепочка (пример: shiba)

1. Bootloader → ядро + `init_boot` (стоковый first-stage) + наш `vendor_boot`.
2. First-stage: `IsRecoveryMode()` (`access /system/bin/recovery`, открыт)
   → exec нашего `/system/bin/init` — это **статический стаб**, не настоящий init.
3. Стаб: снапшот рамдиска (для reflash) → анпак LGZ-кластера → exec
   настоящего `init.real`. Фейл в recovery-режиме = `reboot bootloader`.
   Legacy-путь в `SecondStageMain` пропускается (handoff по `init.real`).
4. `early-init exec` → `recovery-pixel-boot init` (резолв девайса, пропсы,
   hinge-детект).
5. `on init exec` → `setup-temp`; стартуют Trusty/keymint/weaver-сервисы.
6. `on boot`: USB-конфиг → `recovery-pixel-boot boot` (синхронно, init
   ждёт — модули до GUI) → `start otg_enable` → `patch_dwc3=1` →
   `start otg_auto`.
7. Сервис `recovery` → GUI → `twrp.cpp` дёргает пустой `runatboot.sh`.

## Init-стаб (`include/recovery-init-stub/`)

Статический C (`static_executable`, без логов) PID 1 на месте
`/system/bin/init`; настоящий init едет **внутри LGZ-кластера** как
`init.real` (`init` в exclude-списке, `init.real` — нет; свап делает
колбэк до паковки и манифестов).

- **Первое invocation** (`init.real` отсутствует): встроенный снапшот
  (`snapshot.c`, порт Rust-версии; Rust-бинарь `ramdisk_snapshot`
  оставлен фолбэком) → `lgz decompress` → exec `init.real`.
- **Последующие** (`second_stage`, `init.real` на месте): мгновенный
  passthrough с сохранением argv/env.
- Распаковка обязана случиться до `selinux_setup`: `init.real`,
  sepolicy и пропсы должны лежать на месте до `SetupSelinux`/`PropertyInit`.

Зачем стаб вообще: файлы, нужные **до** анпака (сам анпаковщик, `recovery`,
`*.rc`, манифесты), не могут ехать внутри кластера. Стаб — минимальный
исполняемый мостик между first-stage и кластером. Пакет:
`PRODUCT_PACKAGES += recovery_init_stub`.

## LGZ-кластер (сжатие рамдиска)

Файлы рамдиска пакуются в один solid UCOMP02-кластер `/lgz_cluster.lgz`
(хост-паковщик `lgz_compress_full_x64`, девайс-распаковщик
`lgz_compress_lean_arm64`), который стаб распаковывает до
PropertyInit/SELinux/парсинга RC. Зипы едут внутрь прозрачно (ingest
целиком, восстанавливает `lgz decompress`).

Политики (`LGZ_POLICY`, дефолт `dirs`):

- `dirs` — только каталоги из `LGZ_PACK_DIRS` (скрипты, библиотеки,
  шрифты, прошивочные бинари; `.ko` и `.zip` лежат открыто). Расширение
  набора: дописать каталог — в логе видно `Entries packed`.
- `all` — всё кроме исключений (максимальное сжатие).

`LGZ_EXCLUDE_LIST` побеждает всегда. Форматы записей:

| Запись | Смысл |
|---|---|
| `"recovery"` | basename в любом месте |
| `"*.rc"` | по суффиксу |
| `"twres/dir/file.ext"` | точный рамдиск-путь |
| `"twres/subdir/"` | всё дерево под каталогом |

Правило: всё нужное **до** анпака (стаб, `lgz`, `recovery`, `*.rc`,
манифесты) — только в exclude. Init-замыкание (linker/libc/…) со
стабом едет **внутри** кластера. `LGZ_DROP_LIST` в колбэке удаляет
мёртвый вес из стейджинга до паковки (проверка `readelf NEEDED` +
`strings` на dlopen обязательна; CJK-шрифт и installer-zip'ы дропать
только для тестовых сборок).
