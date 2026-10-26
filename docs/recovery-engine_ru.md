> [English version](recovery-engine.md)

# Rust-движок `recovery-pixel-boot`

`Гайд v3 · 2026-10-26 · include/recovery-pixel-boot/`

Один статический мультиколл-бинарь (только `std` + `libc`), делающий весь подъём девайса. Железное правило: **Rust не exec'ит внешние бинари кроме как через единый чокпойнт** `stage.rs`, дёргающий стадии `pixelrunatboot.sh` (там живут `resetprop`, `bootctl`, `siw`/`iw`, `unzip`, `mount`, `setenforce`). Единственный `fork()` вне стадий - Magisk-линк демон-зеркало в `boot.rs` (без exec). Протокол: argv стадия + аргументы, результат в stdout, диагностика в `/tmp/recovery.log`, exit 0 = успех. Каждая строка движка зеркалится и в `/dev/kmsg` (`ko_picker::log_msg` → recovery.log + dmesg).

## `init` - личность девайса (early-init exec)

Работает **до** фиксации геометрии TWRP (`twrp.cpp SetDefaultValues` / `data.cpp`) и до открытия DRM в minui, так что первый кадр сразу правильный. Лог: `/dev/logs/runatinit.log`.

1. Логируются ядро (`/proc/sys/kernel/osrelease`), `ro.hardware`, суффикс слота.
2. Резолв кода девайса - порядок `ro.product.device` → `ro.product.name` → `ro.hardware` (`resolve_device_code`). Железо уровня семьи (`"malibu"`) одалживает первую секцию своей семьи, чтобы свап ниже всё равно сработал.
3. Стадия `props-apply <family> k=v...` (`resetprop` + `setenforce 0`; пары из `/pixelrunatboot.json`, смердженного на сборке).
4. Повторный резолв кода - применённые пропсы могли поправить `ro.product.device` (бутлоадерный `ro.hardware` бывает семейным). Скорректированный код нужен только свапу флагов ниже.
5. **Второй проход семейного свапа AIO** (первый - C-стаб `aioswap.c`): любой живой файл с маркером `AIO LIVE placeholder` заменяется из кита - `/system/etc/recovery.fstab.<fam>`, `/system/etc/twrp.flags.<fam>`, `/system/etc/recovery.wipe.<fam>`. Идемпотентно пофайлово (токены `already-swapped` / `swapped` / `no-kit` в логе). Манифест кита - `/system/etc/aio/families.txt` (`<fam>:keymint=:usbctrl=:usbpath=`), карта девайсов - `/system/etc/aio/devices.txt`.
6. Геометрия экрана: hinge-зонд (EV_SW `SW_LID`/`SW_TABLET_MODE` по `/dev/input/event*`; неизвестен/нет = безопасный `ClosedCover`), затем `DOF_SCREEN_W/H` + `DOF_PROGRESSIVE_SCALE` (+ `DOF_THEME` когда заданы `wide_theme`/`inner_theme`, + `DOF_STATUS_H` всегда, + `DOF_MAX_BRIGHTNESS` когда `max_brightness` > 0). Обычный `setprop` - форк `resetprop` не нужен (не `ro.*`).
7. Оверрайд девайса: `/system/etc/<device>.twrp.flags` (из `devices/<codename>/`, опционально - сейчас не шиппится ни одного) побеждает семейный дефолт.
8. `fix_twrp_flags`: (а) UFS-фолбэк - запечённые префиксы `/dev/block/platform/<addr>.ufs/`, отсутствующие на девайсе, перетыкаются на живой `*ufs*` каталог (существующие не трогаются); (b) by-name prune - строки, чей раздел отсутствует и как `<part>`, и как `<part>_a` в `/dev/block/platform/*/by-name`, дропаются (`symlink_metadata`: by-name записи висят пока идёт ueventd coldboot; пока by-name не заселён - скип вообще).
9. Скан `Magisk-*.zip` в `/system/bin` → стадия `magiskboot-unpack` (или варнинг `no Magisk zip, skipping extraction`).
10. `setprop servicemanager.ready true`. (Zip-рестора нет: solid UCOMP02-кластер глотает `*.zip` на сборке, `lgz decompress` их возвращает; старый поток `/lgz_zip_manifest.txt` мёртв.)

<details>
<summary>⚙️ Остальные субкоманды - что запускается когда</summary>

| Субкоманда | Триггер | Что делает |
|---|---|---|
| `boot` | `exec` на `on boot` (init ждёт - модули до GUI) | порядок `boot` ниже |
| `otg-patch` | сервис `otg_enable` | сначала USB-gate (блок → `sys.usb.patch_dwc3=0`, девайс-режим); затем семейный патч, выставляет `sys.usb.patch_dwc3=1` |
| `otg-auto` | триггер `patch_dwc3=1` | семейный VBUS-демон, не возвращается; гейт перепроверяется (блок → парковка на часовом сне, девайс-режим) |
| `setup-temp` | `exec` на `on init` + рефреш из `boot` | Симлинк `/dev/thermal_cpu`: exact-путь → `soc_therm` → авто-список → zone0 |
| `torch on\|off` | `OF_FL_PATH1=cmd:/system/bin/recovery-pixel-boot torch` из GUI (`vendorsetup.sh:171`) | LM3644: дискавери зарегистрированного I2C-клиента (`-0063`, `I2C_SLAVE_FORCE` через `i2c_transact`) → generic LWIS-DT фолбэк (`google,lwis-i2c-device`, напр. malibu/laguna и gs101/raven когда GPIO-промах) → GPIO v1 uAPI; нативные ioctl, без форков (запись `USBSW 0x93←0x09` относится к TCPC-патчу в `usb`/`i2c.rs`, не к фонарику) |
| `dm-state` | отладка | Рефреш `/tmp/fox_dm_state` раз + печать |
| `dm-watch` | демон из init rc | Снапшот-only вотчер dm-правды (никогда не мэппит/размэппит) |
| `install-rom <args>` | оставлен как алиас (тонкая обёртка, `install.rs`) | `twinstall` exec'ит `/system/bin/fox-hsinstall` напрямую, не через алиас (status-fd pipe наследуется); exit 2/5 держат стоковый контракт |

</details>

<details>
<summary>🚀 Порядок `boot` - вся последовательность (boot.rs)</summary>

1. Busybox дампится в `/dev/.fox_bb/busybox` (апплеты переживают замены пакетным менеджером); `chmod 777` всего в `/system/bin`.
2. Детект слотов: `ro.boot.slot_suffix`, фолбэк стадия `slot-detect` (`bootctl get-current-slot` → суффикс, `0`→`_a` / `1`→`_b`).
3. `susfs_fix`: `ko_try_load("susfs_rename_fix", /proc/susfs_rename_fix)` - фикс паники fast-symlink rename (след в proc = успех, монолит = игнор).
4. Загрузка конфига девайса (неизвестный коднейм = пусто, скип как старая `*` ветка, никогда не фатально) + firmware и тач, оба под гейтом `touch_modules` не пуст: `fw-fetch <part_vendor> <sfx> <slot>` (siw|iw, оба слота), затем **тач** (единый гейт в `boot.rs`).
5. Тач, 5 источников строго по порядку (параллельные ko-fetch потоки покорёжили бы общие `/dev/ko_stage` + `/dev/stage_*.img`): `touch_unpack` (`/<code>.lgz` из пейлоада → наполняет OFOX-пребилд каталог) → стоковый `vendor_dlkm` текущего слота → противоположного → фолбэк `/system/modules_touch` → живой `/lib/modules` (точный vermagic, обходит блоклист first-stage init прямым `finit_module`) → OFOX-пребилд (`/system/lib64/modules/touch`, лучший скор первым). Провайдерный preload (`part_sysdlkm` + `preload_modules`, напр. `pwrseq-core` перед `lwis`) идёт первым когда настроен - сегодня не настроен ни у одного девайса, путь только варнит.
6. Форс-фолбэк: везде строгий `finit_module(flags=0)`, кроме одного прохода `IGNORE_VERMAGIC/MODVERSIONS` (`0x3`) по лучшему OFOX-кандидату при полном строгом фейле (только свой `.ko` - kprobe/procfs ABI-стабильны в 6.1.x; **никогда** для стоковых вендорских модулей). `EEXIST` = успех везде.
7. Вердикт: полный промах по всем источникам поднимает `fox.force_hw_gui=1` (GUI опрашивает его через хук `fox.force_hw_gui`, переключается на клавиши VOL+/VOL-/POWER + модалку - через `setprop`, никогда через ORS FIFO) - **но только на доказанно-модульных ядрах** (кешированный вердикт монолита). Монолит/частичный (Sultan-стайл) промах терпит: драйверы могут быть вкомпилены под другим именем.
8. Диета RAM: `prune_touch_modules` сносит распакованные снапшоты, которые эта загрузка НЕ инжектила (архив `/<code>.lgz` остаётся - reflash его дописывает), затем рефреш термала (`setup_temp` повторно: топология зон изменилась когда встали `vendor_dlkm` драйверы).
9. `usb_modules` (11 девайсов: наборы gs101/laguna/malibu; zuma/gs201/zumapro ничего не надо - Samsung-стек самоавтостартует из рамдиска): тот же ko-fetch механизм из `part_usb` (дефолт `vendor_dlkm`); промах роняет OTG в девайс-режим с варнингом.
10. Подпинывание подсветки: любой узел `/sys/class/backlight/*` ниже 10% от max поднимается до 50% (поздний проб панели vs one-shot дискавери TWRP; no-op когда TWRP уже выставил норму).
11. dm-правда: `ensure_keep_mapped(part_touch, sfx, slot)` оставляет kept LP-узел (`vendor_dlkm`, `vendor`) для TWRP + `dm::refresh("boot")` публикует `/tmp/fox_dm_state`.
12. Заметка про torch-узел (`/dev/lwis-flash-lm3644` есть или LWIS-DT напрямую на malibu/laguna), стадия `meta-fix` (чистка `/metadata/ota`), Magisk-линк: `Magisk-*.zip` копируется в `/FFiles/OF_Magisk/` (+ в `/sdcard/Fox/FoxFiles/` сейчас и навсегда через форкнутый демон каждые 2 с когда смонтируется `/data`).

</details>

<details>
<summary>🎯 Скоринг `.ko` (ko_picker.rs) - формула</summary>

- Окружение из `uname -r` (`6.1.157-android14-11` → major/minor/patch) + pagesize через `sysconf` + Android API из uname или SDK-пропса (`33`→13 … `36`→16).
- Токены имени + каталогов: `android<N>`, `X.Y`, полный `X.Y.Z`, голый patch-каталог (`touch/6.1/157/…`) только после увиденного в порядке пути `X.Y`.
- **Жёсткий фильтр:** pagesize - `16k`-кандидат на 4K-ядре (или наоборот) отрезается сразу.
- Скор: совпадение ветки ядра **1000** (мимо 10, последний шанс) + точный патч **300** (иначе близость ≤200, nearer wins) + API **200** + консистентность файл/каталог **50** + каталог `4k`/`16k` **50**. Мимо патча - не реджект (6.1.x кросс-грузится, полевое доказательство: модуль `.145` на ядре `.157`).
- Присутствие = `/proc/modules` **или** `/sys/module` (вкомпиленные в первом не видны); написания через дефис/подчёркивание объединяются. Загрузка всегда raw-`finit_module` syscall (в бионике нет обёртки на API 34).
- **Монолит, 3 уровня, кеш на загрузку:** нет `/proc/modules` (настоящий монолит) → пустой lsmod → канареечный зонд: `empty_checker_module` из `/system/lib64/modules/probe/` (та же раскладка kver/android/pagesize), вердикт = **след** `/proc/empty_check_gki`, а не код возврата (Sultan подделывает RC=0 и пустой lsmod). Кастомные модули (шимы, susfs) на монолите игнорятся; ветка тача промахи там терпит.
- Запись инжектов за загрузку `/tmp/fox_ko_loaded` (дедуп, дефис/подчёркивание едины) - зависимые гейтятся на «уже инжектили» без повторного прогона цепочки.

</details>

<details>
<summary>🔌 USB - два механизма, один предикат (usb/)</summary>

- Личность семьи: `ro.recovery.soc_family` (штампуется на early-init) побеждает, таблица коднеймов `ro.hardware` - фолбэк (`aio`/пусто = `Unknown` → стаб-путь, host никогда не форсится).
- **Поколения Exynos** (zuma/gs101/gs201/zumapro): `otg_host_shim` (kprobe на `dwc3_otg_host_ready`, заменяет отсутствующий AoC-зонд) + `OTG_ID` + форсаж VBUS `CHARGER_MODE`, VBUS-демон.
- **Google-кремний** (laguna/malibu): `aoc_vote_shim` (элекция `USB_DR_EL` `cast_vote("AOC",1,1)` - вообще без вендорского стейджинга); элекция самосвитчится, демон только держит голос + проверяет + ведёт `/dev/block/otg-usb`.
- **Предикат успеха** (роль сама по себе - никогда): downstream `role == host` **и** source power_supply `online == 1` **и** резолвящийся `/dev/block/otg-usb`.
- `zuma` - lean TEST-ветка (алгоритм R11): 6.1 = kprobe-шим + форс-загрузка (пребилд штампован 6.1.176 против 6.1.157 на девайсе - исключение для своего шима); 6.12 = нативный путь `usb_role_switch`, шим по дизайну не нужен (AOC-проверка, которую патчил шим, там просто отсутствует). Что осталось из полевого опыта: отвязка UDC с `"\n"` (0-байтные записи не долетают до configfs), `ffs.ready` пишется один раз, явный ребйнд UDC на свитче в девайс (init-триггеры edge-based), стабильный симлинк `/dev/block/otg-usb`, `usb power_supply` = VBUS-сенсор, `CHARGER_MODE` = gvotable-элекция.
- `laguna` (Tensor G5): отвязка UDC + role-switch `host` + форсаж VBUS gvotable `CHARGER_MODE`, VBUS-gated демон. Элекция `google-role-sw` наблюдается, но на неё **не полагаются** (`role` липнет пока Type-C `data_role` остаётся `[device]` - декоративно). Полевое доказательство на mustang CP3A: 6.6 stable + 6.12 beta, один ручной путь. Fox (G5+): контроллер порта - SPMI (`spmi-max77759tcpc`), не I2C - I2C-скан мимо по дизайну, стоковый драйвер сам ведёт data path, распознание SPMI-девайса считается успехом.
- `malibu` (Tensor G6): тот же ручной путь + `aoc_vote_shim` (голос `USB_DR_EL`, assert когда `/proc/aoc_vote_ready=1`); предпочитается DWC3-запись контроллера (`a200000.usb3-role-switch`). I2C-патча TCPC нет (шина SPMI).
- Фейл везде одинаков: вежливый девайс (adb цел), никогда притворный host.

</details>

<details>
<summary>🛡️ USB-guard - почему host-режим иногда не включается (usb/gate.rs)</summary>

- Ядро новее прошивки **паникует**, когда кастомный OTG-стек форсит VBUS в host (видели в поле). Перед любой загрузкой шима или записью питания гейт сверяет патч запущенного ядра с `vermagic`-триплетами прошивочных модулей, прочитанными из raw-байт `.ko` (кэп 1 МиБ, игла `vermagic=` - ядро никогда не сообщает под что собран загруженный модуль), голосование большинством по ≤32 модулям (в стоковых деревьях есть залётные, напр. один 6.1.75 среди 6.1.157).
- **Пустой lsmod** (трогать нечего - ничего версионного в игре нет) - проверка бессмысленна, идём дальше. **Монолитное ядро** (модульных syscall нет) - инжектить нечего, идём. **Нет читаемых стоковых `.ko`** - доказать дурость нельзя: идём дальше, но с явным варнингом в логе (fail-open). **Патч большинства прошивки совпал с запущенным** - идём. Иначе **БЛОК**: нет сервисов, нет записей VBUS - обычный девайс-режим (adb цел).
- Порядок скана: gs101 сначала `/lib/modules` (first-stage распаковал туда dlkm-фрагмент `vendor_boot`), остальные сначала `vendor_dlkm`; `/system/**` не сканируется никогда (свои шимы кросс-версионны по дизайну). Перепроверяется в `otg-auto`: ручной `patch_dwc3=1` на gated-дереве паркует демон вместо управления VBUS.
- Гейт читает патч-триплеты с диска; он дополняет (не заменяет) собственные вердикты загрузчика `finit_module`.

</details>

<details>
<summary>🔥🌡️ Фонарик + термал + dm-правда</summary>

- **Фонарик** (`torch.rs`, `i2c.rs`): полностью нативно, форков `gpioset`/`i2cset` нет. (1) зарегистрированный I2C-клиент (`/sys/bus/i2c/devices/*<match>`, дефолт `-0063`, `I2C_SLAVE_FORCE`, `USBSW 0x93←0x09`); (2) LWIS-фолбэк (malibu/laguna `google,lwis-i2c-device` DT-узел); (3) GPIO v1 uAPI (определён локально - в libc нет gpio ioctl). Хук `OF_FL_PATH1=cmd:…` из GUI.
- **Термал** (`temp.rs`): CPU BIG-зона - хотспот (~90 °C на холодном девайсе), виджету TWRP туда нельзя - HAL-стайл репрезентативный сенсор (`soc_therm`) вместо любого среднего; нумерация зон плывёт по поколениям Tensor, а именованный сенсор стабилен. Exact-path режим когда задан `thermal_temp_path`, иначе авто по `thermal_zone_types`; повторный прогон из `boot` после вставания `vendor_dlkm`.
- **dm-правда** (`dm.rs`): `/tmp/fox_dm_state` (`<name> <dm-target>` построчно, заголовок `# v1 epoch=`) - время жизни как у dm-неймспейса (tmpfs). `dm-watch` опрашивает `/dev/block/mapper` и переписывает при изменении, **снапшот-only по дизайну**: никогда не мэппит/размэппит, так что не воскресит узел, который `Unmap_Super_Devices` только что снёс при формате. `KEEP_PARTS = ["vendor_dlkm", "vendor"]` (first-stage их сам не мэппит; без kept-узла `/vendor_dlkm` вообще не смонтируется - полевой кейс gs101 raven). TWRP новая логика не нужна: стабильных узлов + файла правды хватает. Shell-стадии рефрешат как trap-on-EXIT фолбэк.

</details>

<details>
<summary>🐚 Shell-стадии - полный список (pixelrunatboot.sh)</summary>

- `props-apply <family> k=v...` (`resetprop` × N → в stdout счёт; `setenforce 0`); `slot-detect` (`bootctl`, печатает `_a`/`_b`); `dm-map <part> <sfx> <slot>` (мэп одного LP-узла для TWRP); `ko-fetch` (`siw read` → `iw read` с фолбэком map+mount+loop-connect, оба слота; depth guard: только `lib/modules/*.ko` - подкаталоги pagesize с чужими CRC тенили плоские модули на 6.12); `fw-fetch` (firmware → `/vendor/firmware`); `magiskboot-unpack` (`unzip` → `busybox` в `/system/bin`); `meta-fix` (ждёт блочный узел, чистит `/metadata/ota`); `touch-unpack <code>` (`/<code>.lgz` → `/`, плохой коднейм реджектится).
- Хелперы: `_siw_stream` (один глобальный неймспейс - все переменные `local`, старый клоббер стейджил образ сам в себя), `_iw_extract` (голый рекурсивный `iw read -f` + grep-фильтр), `_siw_map`/`_siw_unmap` (размэппятся только свои мэппинги - чужие first-stage/TWRP/lptools узлы read-only), `_siw_map_copy`/`_siw_connect_copy` (loop-connect read-only mount для не-`.ko` файлов), `_dm_target`/`_keep_listed`/`_keep_node_mapped`/`_dm_snapshot` (mapper-правда + keep-список, зеркалится Rust-модулем `dm`).
- `siw`/`iw`/`lptools_new` - статические arm64-утилиты в рамдиске. Стадии используют `siw read` → `iw read` (потоковые чтения без монтирования) + DM-мэппинг через `siw map` с settle-wait и siw-UUID владением; `lptools_new` лежит как запасной бинарь, но его не дёргает ни одна стадия. Mount-hunt существует только для не-`.ko` файлов (aocd/библиотеки), которые ko-fetch нести не может.

</details>

<details>
<summary>⚡ Скоростная установка (`fox-hsinstall`) + живое переключение слота</summary>

- Opt-in, по умолчанию выкл, только A/B-девайсы. Тумблера два: **High-speed flash** на странице подтверждения прошивки zip, а с beta_1_2 ещё и на экране **sideload** (до этого HS sideload-пакеты отклонял по дизайну - exit 5, стоковый путь; чтение sideload теперь заточено под USB: последовательные чтения, переиспользуемый fd, глубокий readahead - пределом стал кабель, а не движок). `twinstall` детектит payload-OTA и запускает `recovery-pixel-boot install-rom <zip>` по тому же updater pipe (`progress` / `set_progress` / `ui_print`); стоковый `update_engine` остаётся фолбэком. Полевые замеры на shiba, zip EvoX ~3.3 ГБ: HS sideload 3:05 против 8:30 стока, локальная HS-установка 0:50 против 3:50 стока.
- Вердикты движка: mismatch `pre-device` vs `ro.product.device` = жёсткая ошибка, без фолбэка. Инкрементальная OTA = exit 2, стоковый путь. Нет хелпера `siw` = exit 5, стоковый путь (`lptools_new` лежит в рамдиске, но ни один путь кода его не требует). На шифрованных девайсах HS раньше падал - починено, шьёт (тег тестера test_9_6).
- Поток: `siw info -f json` живое состояние → fit-лестница (`FITS_AS_IS → KILL_ONE → KILL_TWO → KILL_MANY → WIPE_ALL → IMPOSSIBLE`, снос от largest-first из текущего слота) → бэкап критических весов 2+4 в `/tmp` (super/modem исключены) → однопоточная пересборка групп через `siw` (unmap, remove + create в обоих мета-слотах, map writable) → батчи 1→2→3→4 по LPT, sha256 на поток + барьер на батч → при фейле добить батч, откатить (для weight-1 super-батча скип, критичного ещё ничего не забэкаплено), exit 1 (слот сам движок не переключает) → `fsync` + `sync`. `/data` никогда не форматирует: триггер downgrade-формата осознанно не копируется.
- Что видно живьём: короткие строки Step 1 - Step 4 вместо спама verify (ошибки видны полностью); живой бар заливает полосы 65/75/85/95, а каждый тихий хвост сообщает `verifying <partition>` вместо замёрзшей полоски.
- После HS-прошивки установщик **всегда выставляет свежим активный слот** (тег тестера test_9_9: слот раньше оставался на месте и телефон падал в bootloader; найдено на Pixel 6, но по коду могло ударить по любому девайсу).
- `fox-hsinstall` собирается cargo (`include/source/`, вне Soong), статический пребилд в `recovery/root`. Операции с LP идут через форкнутый `/system/bin/siw`: kernel-guard тихо режет raw-записи в super от любого другого бинаря, прямые записи `liblp` невозможны по дизайну. Кодеки - чистый Rust (`lzma-rust2` + `ruzstd` + `crabz2`). Прогресс - в stdout (updater pipe: staged user lines, `set_progress`, `log`), twinstall ретранслирует живьём в консоль GUI.
- **Живое переключение слота** (тумблер `tw_live_switch`, по умолчанию выкл, инертен; висит в опциях установки + на странице sideload): убивает перезагрузку между прошивкой и аддонами - сессия переезжает на свежий слот по окончании установки, так что цепочка **ROM → GApps → Magisk** шьётся за один заход (GApps ложатся в свежую систему, Magisk патчит свежий boot; то же для ядра после ROM). Вкл = автопереключение без ребута; выкл = классика (прошил, потом ребут для свитча). Успех показывает сообщение «слот переключён живьём, следующий ZIP можно шить без ребута» (раньше было тихо).
- Дисциплина слота с включённым тумблером: цель нормализуется перед каждой прошивкой - сидя на A, всегда шьёт в B, никогда в загруженный слот, так что очередь аддонов остаётся консистентной; прошивка, нацеленная в загруженный слот, переключается обратно на загруженный слот с варнингом и продолжается (файл-менеджер и sideload); сам движок загруженный слот отклоняет как бэкстоп; внешние смены слота подхватываются сами. Внутренности: сверка с `bootctl` на старте установки, снапшот загруженной правды на старте (`bootctl` спуфабелен), перепривязка spoof-файлов, мэп нового слота через `siw` до того, как rescan сбросит объекты. Срабатывания гарда нормализуются вместо отказа во второй прошивке. DFE-совместимые маркеры source/target слота. Mount-before-refresh в живом пути. Порядок тумблеров на экране установки сейчас: Auto reflash, High-speed flash, Live switch, затем проверка подписи и размонтирования.

</details>
