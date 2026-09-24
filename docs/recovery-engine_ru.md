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

`siw`/`iw` — статические arm64-утилиты (1.1M/1.4M): потоковое чтение
разделов без монтирования (`siw read`) и DM-маппинг (`siw map`,
замена `lptools_new --map`: `/dev/block/mapper/<имя>` через DM ioctl).
Стоковый `runatboot.sh` — пустой хук OFox (дёргает `twrp.cpp`); точка
для аддонов.
