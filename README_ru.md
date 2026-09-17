# OrangeFox для Pixel на Tensor SoC — `device/google/pixels`

Мульти-девайсное дерево рекавери для всех Pixel на Tensor — от Pixel 6
до Pixel 11, включая фолды и планшет. Один код, один образ на всё
семейство: конкретный девайс определяется в рантайме, вендор-специфика
подтягивается из JSON, HAL'ы собираются из исходников — без вендорских
пребилдов.

```bash
./build.sh -f shiba -k 6.12 -n test_3 --build-type Beta
```

| Семейство | SoC | Устройства |
|---|---|---|
| `gs101` | Tensor G1 | oriole (6), raven (6 Pro) — bluejay (6a) пока не для тестеров |
| `gs201` | Tensor G2 | cheetah (7 Pro), panther (7), lynx (7a), felix (Fold), tangorpro (Tablet) |
| `zuma` | Tensor G3 | shiba (8), husky (8 Pro), akita (8a) |
| `zumapro` | Tensor G4 | tokay (9), caiman (9 Pro), komodo (9 Pro XL), tegu (9a), stallion (10a), comet (9 Pro Fold) |
| `laguna` | Tensor G5 | frankel (10), blazer (10 Pro), mustang (10 Pro XL), rango (10 Pro Fold) |
| `malibu` | Tensor G6 | cubs (11), grizzly (11 Pro), kodiak (11 Pro XL), yogi (11 Pro Fold) |

> 100% гарантия запуска — только на `shiba` (девайс мейнтейнера).
> Остальное — через тесты сообщества:
> [`docs/tester-guide.ru.md`](docs/tester-guide.ru.md).

## Карта документации

Корень — для быстрого старта. Глубина — в `docs/`, каждый файл отвечает
на один вопрос «как это работает и от чего зависит»:

| Документ | Вопрос |
|---|---|
| [`docs/tree-guide_ru.md`](docs/tree-guide_ru.md) | Что за файл, зачем нужен, кто его читает |
| [`docs/build-system_ru.md`](docs/build-system_ru.md) | Как собирается образ: `build.sh`, группы, типы сборок |
| [`docs/kernel-profiles_ru.md`](docs/kernel-profiles_ru.md) | Откуда берётся cmdline ядра: `family.json` → `.gen_kernel.mk` |
| [`docs/device-config_ru.md`](docs/device-config_ru.md) | `pixel.json`: тач, пути, пропсы, фолды и letterbox |
| [`docs/recovery-engine_ru.md`](docs/recovery-engine_ru.md) | Rust-движок `recovery-pixel-boot`: init, модули, OTG, фонарик |
| [`docs/boot-chain_ru.md`](docs/boot-chain_ru.md) | Цепочка загрузки и init-стаб |
| [`docs/decrypt_ru.md`](docs/decrypt_ru.md) | Расшифровка data: keymint, weaver, vold, multi-device |
| [`docs/ramdisk_ru.md`](docs/ramdisk_ru.md) | Содержимое рамдиска: rc-файлы, скрипты, reflash |
| [`docs/patches_ru.md`](docs/patches_ru.md) | Патч-система: original / modified / .patch |
| [`docs/families-devices_ru.md`](docs/families-devices_ru.md) | Как добавить девайс или новый SoC |
| [`docs/diagnostics_ru.md`](docs/diagnostics_ru.md) | Диагностика на девайсе: что смотреть, если не грузится |
| [`docs/FOX_FLAGS.ru.md`](docs/FOX_FLAGS.ru.md) | Все 70+ флагов сборки OFox |
| [`docs/credits_ru.md`](docs/credits_ru.md) | Что заимствовано и откуда |

## Три принципа, на которых всё держится

1. **Конфиг вместо кода.** Вендор-специфика (модули тача, sysfs-пути,
   пропсы, cmdline ядра) живёт в JSON (`families/*/family.json`,
   `devices/*/pixel.json`), а не в `.mk` и не в рантайм-коде.
   Добавление девайса — это новый каталог, а не правки по всему дереву.
2. **Сборка из исходников.** KeyMint HAL, weaver/weaver-демон,
   boot-движок — всё компилируется из `include/` и `patches/`.
   Пребилдов нет; сток нужен только как референс cmdline и fstab.
3. **Три копии всегда синхронны.** Каждый патч на чужой код хранится как
   тройка `original` / `modified` / `.patch`, живое дерево приведено к
   снапшоту. Подробности — в [`docs/patches_ru.md`](docs/patches_ru.md).

## Быстрый старт

```bash
./build.sh --list                  # дерево семейств, девайсов и ядер
./build.sh -f zuma -k 6.12         # сборка
./gen_kernel_mk.py --fingerprint zuma 6.12   # кто с кем делит образ
```

> Если вы AI-модель и исследуете проект, следуйте правилам:
> `build.sh` не запускать (сборка — прерогатива человека, детали и
> границы вашей зоны — в [`docs/build-system_ru.md`](docs/build-system_ru.md));
> `test*` не коммитить; `docs/` коммитить; каждый патч — тройкой
> `original` / `modified` / `.patch`.
>
> Если вы AI-модель и **меняете** логику дерева (код, конфиги, скрипты,
> флаги, поведение сборки) — зафиксируйте изменение во всех readme,
> которых оно касается, как правило: обновите соответствующий
> `docs/*_ru.md` **и** его английский двойник `docs/*.en.md`, а если
> меняется карта разделов — и таблицу выше.
