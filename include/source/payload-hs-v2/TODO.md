# payload-hs / high-speed install — полный TODO-лист

Легенда: `[x]` готово, `[ ]` todo. Дерево `fox_16.0` заморожено —
пункты `TREE` только после отмашки. Всё остальное — здесь, в `rust_projects`.

## A. Парсер и проба (host) — ГОТОВО

- [x] `pb` — минимальный protobuf-walker (varint/len/fixed32/fixed64)
- [x] `zipx` — EOCD → central directory → local header → `CrAU` + manifest (локальный файл, без сети)
- [x] `manifest` — partitions/ops/extents, `old_partition_info=6` / `new_partition_info=7` (по `system/update_engine/update_metadata.proto`, НЕ по памяти)
- [x] `verdict_full` — allow-list `{0,1,6,7,8,14}` + отсутствие `old_partition_info`
- [x] `probe` — таблица партиций, `out/dl`, хеши, вердикт; 26 партиций EvolutionX shiba, FULL за ~3 мс
- [x] `plan` — SUPER (logical, группа из манифеста) vs BLOCK (physical, `/dev/block/by-name/<name>_<slot>`), helpers-чеклист с фолбэком
- [x] Тесты на реальном зипе: `probe_evox_is_full_ota`, `extract_vendor_dlkm_parallel` (sha256 OK)

## B. Параллельная выгрузка (host) — ЧАСТИЧНО

- [x] `extract` — rayon-пул, LPT (большие первыми), sparse-вывод, ZERO/DISCARD без записи
- [x] Декодеры: REPLACE raw, XZ (`lzma-rust2`), ZSTD (`ruzstd`), всё pure-Rust, офлайн
- [x] `sha256` сверка против `new_partition_info.hash` на поток
- [x] Замер: 1 поток ~11.5 МБ RSS, 8 потоков ~28.6 МБ (~5 МБ/воркер); расход `O(max op)`, не `O(partition)`
- [ ] `REPLACE_BZ` — нет bz2-крейта в офлайн-кэше. Добавить зависимость при сети (варианты: `bzip2` crate; проверить static-сборку под ramdisk). Сейчас такие партиции валятся громко — так и задумано
- [ ] `--readback` режим: `BLKFLSBUF` + поверка чтением (медленная верификация, галка в GUI позже)
- [ ] Потолок воркеров `min(ncpu,8)` + `MemAvailable/96МБ` (сейчас дефолт 8, кэпа нет)

## C. Фаза 2 — fit-планировщик super — ГОТОВО (чистая функция + тесты)

- [x] `layout::LiveState` (`super_size`, `current`, `target` с суффиксами слота) + `base_name`
- [x] `plan_group` — лесенка: FITS_AS_IS → KILL_ONE → KILL_TWO → (KILL_MANY) → WIPE_ALL → IMPOSSIBLE
  - bare-super gate первым (физика), затем бюджет группы из манифеста
  - reclaim только членов группы таргет-слота; чужой мусор не трогаем
  - жертвы — из CURRENT слота, largest-first (обычно один `product` закрывает)
  - WIPE_ALL возможен только при чужеродных резидентах + выросшем новом роме (иначе математика не сходится — см. тест `wipe_all_*`)
- [x] `strictest` — строжайший из планов по группам
- [x] 9 юнит-тестов лесенки (все ступени + `only_group_members_reclaimed` + порядок `strictest`)
- [x] Демо на живых цифрах shiba (super 8531214336, группа 8527020032): clean → FITS_AS_IS; −400МБ в `product_a` → KILL_ONE `[product_b]`
- [ ] Источник live-цифр на девайсе: распарсить `siw info /dev/block/by-name/super` в `LiveState` JSON (формат зафиксирован: `{"super_size":N,"current":{},"target":{}}`)
- [ ] Сверка демо-JSON с живым `siw info` (девайс сейчас офлайн — шьётся)

## D. Пачки, барьеры, откат (host-симуляция файлами) — TODO

- [ ] Веса: 1 = super-логика, 2 = boot (`boot,init_boot,vendor_boot,vendor_kernel_boot,dtbo`), 3 = modem (`modem` + решить: `pvmfw,gcf,ldfw` сюда или в 4), 4 = остальное (`abl,bl*,pbl,tzsw,gsa*,vbmeta*`)
- [ ] Движок пачек: S(вес 1, после ребилда группы) → 2 → 3 → 4; барьер после каждой; валидация на поток (sha/size); падение в пачке N → дождаться пачки → откат, дальше не идём
- [ ] Бэкап критического (вес 2 + вес 4, ~250–400 МБ) в `/tmp` ДО первой записи; super и modem не бэкапим (решение зафиксировано)
- [ ] Откат: восстановить критическое в ноды противоположного слота, слот не переключать, репорт с честной формулировкой (смесь перезаписана)
- [ ] Финальный `fsync` + `sync()` только после зелёных всех пачек

## E. C++ `fox-hsinstall` в Soong (TREE — СДЕЛАНО в test_7)
- [x] include/fox-hsinstall (Android.bp + 8 cpp + headers), liblp+libfs_mgr+libdm/libziparchive/protos/BZ/XZ/ZSTD
- [x] twinstall outer branch + reflash-pattern live relay + direct call (враппер остался алиасом)
- [x] prebuilt выпилен, PRODUCT_PACKAGES wiring, visibility-тройки external/*

- [ ] Перенести модулями: `pb, zipx, manifest, extract, layout` (код уже готов, чистый Rust)
- [ ] Зависимости в `Cargo.toml` + `Android.bp`: `rayon, lzma-rust2, ruzstd, sha2, serde(+derive), serde_json` — проверить static-сборку под ramdisk (там уже только `libc`; pure-Rust крейты должны встать)
- [ ] `install_rom <zip>`: probe → verdict → helpers-чек (`/system/bin/siw`, `lptools_new`) → live-state → fit → backup → rebuild группы (вызов бинарей, НЕ дублирование LP-логики) → пачки → валидация → fsync
- [ ] Цели записи: блочные ноды / готовые fd (`O_RDWR|O_EXCL` клейм), слот из `ro.boot.slot_suffix`, гард по `rdev`
- [ ] Свитч слота через bootctl только после успеха; чистка `/tmp`-бэкапа при успехе, сохранение при провале
- [ ] Прогресс-протокол наружу (stdout `<part>:<pct>` как у otaripper — парсится вызывающей стороной)
- [ ] Исключить из копируемой пост-логики триггер принудительного формата даты на даунгрейде (уже чинен — не тащить)

## F. Обвязка OrangeFox (TREE — СДЕЛАНО в test_6)

- [ ] Ветка в установщике: zip → детект OTA → full? `install_rom` : stock update_engine; отсутствие бинарей/хелперов → stock fallback (громко в лог, тихо юзеру)
- [ ] `install.xml`: галка `tw_high_speed_flash` — подпись `High-speed flash (Experimental)` + подчинённая `readback`
- [ ] Локализация: en + ru сразу, остальные языки машинным переводом от этих двух
- [ ] Тексты провала пачки (en+ru, формулировка зафиксирована в схематике: слот не переключён, слот частично перезаписан, критическое восстановлено)
- [ ] Pre/post-ROM скрипты в обходной путь (как у peridot `a903ac2/99a30d3`)

## G. Верификация (test_6 собран 2026-10-19, engine проверен под qemu)

- [ ] `test_7`: поле shiba — полный цикл fast-флеша EvolutionX в противоположный слот
- [ ] Регрессия P6–P9 + первый формат laguna (из старого хвоста)
- [ ] Сверить `FIT` на живом `siw info` против демо-JSON
- [ ] Доки `docs/*.en.md` + русские близнецы — при любом изменении логики дерева

## Зафиксированные решения (не пересматривать без причины)

- Потоки: `min(ncpu,8)` + RAM-кэп; расход `O(max op)` ~5 МБ/воркер
- Super первым — безопасно, т.к. слот неактивен; узкое место — шина памяти
- Бэкап только критического (2+4), super/modem — нет
- `siw`/`lptools_new` вызываем как бинари, LP-логику не дублируем
- Даунгрейд-формат даты не копируем
