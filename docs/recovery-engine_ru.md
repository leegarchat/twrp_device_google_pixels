> [English version](recovery-engine.en.md)
# Rust-движок `recovery-pixel-boot`

Статический мультиколл-бинарь (`include/recovery-pixel-boot/`, `std` +
`liblibc`). Железное правило: **Rust делает только syscalls, внешних
бинарей не форкает** — единственная форк-точка это вызов стадий
`pixelrunatboot.sh` (там, где нужны `resetprop`, `bootctl`, `unzip`).

## Субкоманды

| Субкоманда | Триггер | Что делает |
|---|---|---|
| `init` | `exec` на `early-init` | Резолв девайса → пропсы через стадию → повторный резолв → свап `<device>.twrp.flags` → `twrp.flags`-фикс, распаковка magiskboot через стадию, ожидание `servicemanager.ready`, hinge-детект фолдов (`DOF_*`) |
| `boot` | `exec` на `on boot` (init ждёт завершения — модули до GUI) | preload (`part_sysdlkm`/`preload_modules`, best-effort) → susfs-fix, firmware/модули через стадии + `finit_module`, haptics-PM, magisk-линки + fork-демон, `meta-fix` через стадию |
| `otg-patch` | сервис `otg_enable` | Инжект `otg_host_shim` (скоринг `.ko`), `patch_dwc3=1`, свитч max77759 по I2C; Laguna использует TCPM `preferred_role`/`port_type` и проверяет host/VBUS |
| `otg-auto` | триггер `patch_dwc3=1` | VBUS-демон host/device, живёт всегда; Laguna возвращает dual-role и sink preference |
| `setup-temp` | `exec` на `on init` + рефреш из `boot` | Симлинк `/dev/thermal_cpu`: exact → `soc_therm` → auto → zone0 |
| `torch on\|off` | `OF_FL_PATH1=cmd:/system/bin/recovery-pixel-boot torch` из GUI | LM3644: дискавери I2C + GPIO (v1 uAPI) + devicetree, нативные ioctl |
| `install-rom <args>` | Legacy-вход (оставлен как алиас) | Проброс в `/system/bin/fox-hsinstall` (cargo Rust, см. ниже); установщик дёргает движок напрямую. |

## Модули (`src/`)

| Файл | Зона |
|---|---|
| `main.rs` | Мультиколл-диспетчер argv |
| `config.rs` | Парсер `/pixelrunatboot.json`, дефолты path-ключей |
| `init.rs` | `init`-стадия: резолв, пропсы, флаги, hinge/cover |
| `boot.rs` | `boot`-стадия: preload, модули устройства (включая Laguna `i2c-dev`), firmware, хаптика, magisk |
| `otg.rs` | OTG-патч и VBUS-арбитраж (гейт: без ПК — host-форс; debounce опросов + settle; Type-C controls для Laguna; UDC-rebind в device-режиме) |
| `ko_picker.rs` | Выбор `.ko`: ветка ядра × pagesize × Android-поколение из `uname` (`<mod>_<ver>[_16k].ko` в иерархии `kver/android/pagesize`). Загрузка строго `finit_module(flags=0)` (форс невозможен: `CONFIG_MODULE_FORCE_LOAD=n`), `EEXIST` = успех |
| `props.rs`, `stage.rs` | Пропсы и протокол вызова шелл-стадий |
| `i2c.rs`, `torch.rs`, `temp.rs` | I2C-примитивы, фонарик, термал |

## Шелл-стадии (`pixelrunatboot.sh` + `siw`/`iw`)

Протокол стадий: argv + stdout (результат) + `/tmp/recovery.log`
(диагностика), exit 0 = успех.

| Стадия | Инструменты | Что делает |
|---|---|---|
| `props-apply <family> k=v...` | `resetprop`, `setenforce` | `ro.*`-пропсы (bionic API их блокирует), gs201-флаги |
| `slot-detect` | `bootctl` | Суффикс слота в stdout |
| `ko-fetch <part> <sfx> <slot>` | `siw read` → `iw read` | Все `*.ko` в `/dev/ko_stage/`; fallback `siw map`+mount. Depth-guard: только `lib/modules/*.ko` — подкаталоги pagesize (`16k-mode/`) пропускаются (близнецы с чужими CRC затеняли плоские модули и ломали тач/хаптику на 6.12) |
| `fw-fetch <part> <sfx> <slot>` | `siw`/`iw`, fallback mount | Firmware → `/vendor/firmware/` |
| `magiskboot-unpack <zip>` | `unzip`, `busybox` | boot/busybox в `/system/bin` |
| `meta-fix` | `mount` | Чистка `/metadata/ota` (с ожиданием блочной ноды) |

`siw`/`iw`/`lptools_new` — статические arm64-утилиты: потоковое чтение
разделов без монтирования (`siw read` → `iw read`) и DM-маппинг
(`siw map` с settle-ожиданием и siw-UUID владением, `lptools_new
--map` через системный fs_mgr-стек). Laguna AoC-стейджинг только
по mapper: блок считается существующим лишь как живая нода
`/dev/block/mapper/` со сверкой на dm-девайс (`/dev/block/by-name/*`
не используется никогда — статика, может висеть); порядок:
mount готового → `siw map` → `lptools_new map` → прямое чтение `iw`
(раз за загрузку, последний шанс). Анмапятся только маппинги,
созданные текущим вызовом — чужие (first-stage/TWRP/lptools)
используются read-only. Oneshot `otg-patch` лишь монтирует готовые
ноды: ничего не создаёт, ничего не удаляет.
Стоковый `runatboot.sh` — пустой хук OFox (дёргает `twrp.cpp`); точка
для аддонов.

## Быстрая установка (`fox-hsinstall` + ветка установщика)

Opt-in (галка `tw_high_speed_flash` на confirm-странице, только A/B,
дефолт выкл). `twinstall` детектит payload-OTA и гонит
`recovery-pixel-boot install-rom <zip>` по тому же пайп-протоколу
апдейтера (`progress`/`set_progress`/`ui_print`); стоковый
`update_engine` остаётся фолбэком.

Движок (`rust_projects/hs-install`, static musl arm64 пребилт):
проба zip → `pre-device` vs `ro.product.device` (несовпадение — жёсткая
ошибка без фолбэка) → full-вердикт (инкремент → exit 2, стоковый путь) →
хелперы `siw`+`lptools_new` на месте (иначе exit 5, стоковый путь) →
живое состояние `siw info -f json` → лесенка фазы 2
(`FITS_AS_IS → KILL_ONE → KILL_TWO → WIPE_ALL → IMPOSSIBLE`, жертвы из
текущего слота от больших) → бэкап критического (веса 2+4) в `/tmp`
(super/модем исключены осознанно) → однопоточный ребилд группы через
`siw` (unmap, remove + create в обоих метада-слотах, map writable) →
пачки 1→2→3→4 с LPT-порядком, sha256-валидация на поток и барьер на
пачку → при провале дождаться пачки, откатить, exit 1 (слот движок не
переключает никогда) → `fsync` + `sync`.

При exit 0 установщик переключает слот (`bootctl setActiveBootSlot`) и
завершает штатно. Движок никогда не форматирует `/data`: триггер
формата на даунгрейд осознанно не копируется.

## Движок быстрой установки `fox-hsinstall` (Rust, static musl)

Сборка cargo (`hs-install`, `include/source/`, вне Soong), поставка
статическим пребилтом в `recovery/root` (механизм как у `siw`).
Работа с LP — через форк `/system/bin/siw`: ядерный гард молча режет
сырые записи в super от любого другого бинаря, поэтому прямые записи
через `liblp` невозможны by design. Парсинг манифеста — чистый Rust.
Кодеки: `lzma-rust2` + `ruzstd` + `crabz2` (чистый Rust, без Си).
Поток: проба → гейт девайса (`pre-device`) → full-вердикт (плюс
дельта-сигнал `minor_version`, проверка точного тайлинга) → живое super
(`siw info` JSON) → лесенка fit → бэкап критического (веса 2+4) →
анмап таргета + stale-нод (живой слот не трогаем) → ребилд группы →
пул потоков уровня опкодов (авто `min(ncpu,8)` + RAM-кэп) с per-op
хешами входа → барьер на пачку → `fsync`, перечитка sha256 против
`new_partition_info.hash` (опциональный `--readback` перепроверяет с
носителя) → откат только при падении весов 2/3/4 → `sync`, чистка `/tmp`.
Прогресс идёт в stdout (протокол апдейтера: стейджи для юзера,
`set_progress`, диагностика в `log`) для цикла twinstall, который
ретранслирует его живьём в GUI-консоль (паттерн reflash).
