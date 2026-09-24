> [English version](patches.en.md)
# Патч-система (`patches/`)

Чужой код (TWRP, vold, libfstab, keymint-обвязка…) правится только
патчами. Каждый патч хранится **тройкой** — три копии всегда синхронны:

```
patches/files/
├── original/<путь от корня исходников>   # pristine-файл (как в AOSP/OFox)
├── modified/<тот же путь>                # полный файл с нашими правками
├── patches/<тот же путь>.patch           # unified diff original→modified
├── new/                                  # целиком новые файлы (если нужны)
└── source_snapshot.{json,txt}            # историческая запись миграции (не реестр!)
```

## Правила

1. **Правится `modified/`, `.patch` регенерируется** `diff -U3`
   (`--label a/… b/…`, формат как у соседей), живое дерево приводится к
   снапшоту. Никогда наоборот: `.patch` вручную не пишут (кроме
   backfill-исключений с обязательным dry-run).
2. **Проверка:** `apply_patches.py --check` — везде должно быть `[OK]`
   (совпадение таргета со снапшотом `modified`). Применение:
   `apply_patches.py --apply --root <корень сборки>` (зовёт `build.sh`).
   Апплаер сам находит файлы через `rglob` — регистрировать новые
   тройки нигде не надо.
3. **Живое дерево = `modified`.** Если дерево уже пропатчено (обычное
   состояние), оригиналы снимаются reversal-ом существующих патчей, а не
   копией из дерева. После правки дерево синхронизируется — следующий
   прогон отчитается `already matches modified snapshot`.
4. **Верификация тройки:** применить `.patch` к копии `original` →
   побайтовое равенство с `modified` (`cmp`), плюс `patch --dry-run`.

## Стек инструментов

| Файл | Роль |
|---|---|
| `apply_patches.py` | `--check` / `--apply`, dry-run `patch -p0 --fuzz=3`, детект дрейфа снапшота |
| `patchlib.py` | Библиотека применения, структурные патчи, bypass-режим |
| `sync_from_bakfiles.py` | Пересборка снапшотов из `.bak` |
| `sync_tree.py` (корень) | Умный `repo sync` с сохранением локалок (`-s` сканер, `-d` только дифф, `-c` интерактив, `-f` форс) |

## Что уже покрыто (57 троек)

TWRP: `data.cpp` (letterbox), `action.cpp` (`cmd:`-torch), `gui.cpp`,
`objects.hpp`, `pages.cpp`, `patternpassword.cpp`, темы;
`minuitwrp`: `events.cpp` (вибро; GS101 — импульс `brightness` с таймером, так как `activate` недоступен для записи в recovery), `graphics.cpp`, `graphics_drm.cpp`
(cover-панель), `resources.cpp`; `partition*.cpp/hpp`,
`twrp-functions.cpp`, `twrpRepacker.cpp`, `install.cpp`, `spl_check.cpp`.
Система: vold (`Decrypt`, `MetadataCrypt`, `Weaver1`, `FsCrypt` + headers),
libfstab (`fstab.cpp/.h`), fastbootd (usb_*), keymaster/keymint `Android.bp`,
`keystore2/globals.rs`, update_engine, `init.cpp`/`util.cpp`, Soong/AIDL-бриджи,
`BoardConfigSoong.mk`. GUI: `TW_FRAMERATE`-проброс (`libguitwrp_defaults.go`),
`listbox.cpp` (скролл без переменной).
