> [English version](build-system.en.md)

# Система сборки (`build.sh`)

## Правила для AI-модели, исследующей проект

Если вы AI-модель и работаете с этим деревом, соблюдайте:

- **Не запускайте `build.sh`.** Сборка — прерогатива человека: она долгая,
  чистит `out/` между группами, упирается в лимит `/tmp` (16 ГБ tmpfs) и
  требует решений (профиль ядра, слоты, теги). Ваш запуск может уничтожить
  чужие артефакты или зависнуть на интерактиве.
- **Ваша зона:** готовить дерево, писать код/патчи/доки и проверять
  результат без сборки — `apply_patches.py --check`, `patch --dry-run`,
  `bash -n`, `gen_kernel_mk.py --fingerprint`, побайтовые сверки.
- Не чистите `/tmp/pixels/` и `out/`, пока идёт чужая сборка.
- `test*` — в `.gitignore` (тестовые скрипты не коммитить); `docs/` —
  коммитить.

## Флаги

```bash
./build.sh -f shiba -k 6.12 -n test_3 --build-type Beta --force
```

| Флаг | Смысл |
|---|---|
| `-f, --family TARGET` | Коденейм (`shiba`) или семья (`zuma`). Без флага — интерактивное меню из `vendorsetup.sh` |
| `-k, --kernel VER` | Профиль ядра (`6.1`, `6.12`) из `family.json`. Без флага — интерактивный выбор |
| `--force` | Неинтерактивный режим. Без `-k` — аборт со списком версий (молчаливого дефолта нет) |
| `-n TAG` | Тег в имя образа |
| `--list` | Показать дерево семейств/девайсов/ядер и выйти (ничего не собирает; `[override]` — девайсный `kernels`) |
| `--build-type TYPE` | Тип сборки, дефолт `Stable` (подробности ниже) |

Устаревших упоминаний `-l` (уровень LGZ) в шапке скрипта не использовать —
актуальный набор флагов этот.

## Что происходит (5 этапов)

1. **Резолв цели.** `families/<X>` существует → семья. Иначе читается
   `devices/<X>/device.conf` (`DEVICE`/`FAMILY`) → `DEVICE_BUILD_FLAG=<семья>`.
   Списки строятся из каталогов — хардкода семейств в скрипте нет.
2. **Kernel-профили.** `gen_kernel_mk.py --fingerprint` считает эффективные
   профили и разбивает девайсы на группы с одинаковым хешем. Генерируется
   `families/<fam>/.gen_kernel.mk` (gitignore): `VENDOR_CMDLINE`,
   `BOARD_BOOTCONFIG`, `FOX_KERNEL_VER`. Генерация — **до lunch**, потому
   что `dumpvars` парсит BoardConfig во время lunch. Пустой
   `VENDOR_CMDLINE` = громкий `$(error)`.
3. **Env-файл.** `vendorsetup.sh` (lunch) пишет `.build_platform.conf`
   (семья, UFS-адрес, keymint-тип, LGZ-политика): env не переживает
   ninja recipe-shell'ы, параметры едут файлом. Перед каждой группой
   снапшот `/tmp/pixels/fox_env.sh` обновляется — пост-имидж хук
   вызывается сборкой с вычищенным окружением, и без файла терялись
   `FOX_BUILD_TYPE`/`OUT` (образы `*-Unofficial-*.img` в корне).
4. **Soong.** Собираются `recovery_init_stub`, `recovery-tensor-daemon`,
   `recovery-pixel-boot`, fstabs, KeyMint HAL **из исходников** (тип —
   поле `keymint` в `family.json` → `FOX_KEYMINT_TYPE`). Оверлеи:
   `TARGET_RECOVERY_DEVICE_DIRS` = корень + `devices/*` + своя семья.
   Чужих rc и секций в образе нет.
5. **Колбэк (`--second-call`).** `fox_build_callback.sh` на готовом
   рамдиске (`$TARGET_DIR`): инжект keymint/VINTF по семье, family
   `twrp.flags`, мердж `pixelrunatboot.json` (`[PIXELCFG]`), хирургия rc
   (USB-адрес, отключение чужого keymint), `post_remove_ramdisk`,
   LGZ-пакование (`[LGZ]`), снапшот-манифест и списки для reflash.
   Между группами — обязательная чистка `PRODUCT_OUT`.

## Группировка образов

Девайсы с одинаковым эффективным профилем (пофлаговое сравнение,
порядок не важен) делят **один** образ (`zuma.img`); разошедшийся —
свой (`zuma_husky.img`, подгруппа — `zuma_husky-akita.img`).
Per-device оверрайды сейчас отключены (`_kernels_disabled` в
`pixel.json`) — каждая семья собирается в один общий образ; возврат —
переименовать ключ обратно в `kernels`.

## Типы сборок: Stable vs Beta

| | `Stable` (дефолт, точное совпадение) | Любой другой (`Beta`, …) |
|---|---|---|
| `OF_ADVANCED_SECURITY` | `1` | `0` |
| `adbd` в рекавери | останавливается (`twrp.cpp`), adb мёртв by design | жив, root |
| MTP | автостарт выкл | автостарт вкл |

Следствие: на Stable пароль расшифровки вводится только с экрана;
adb-на-пароле работает только на не-Stable сборках. Тестерам —
`--build-type Beta`.
