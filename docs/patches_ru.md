> [English version](patches.md)

# Патч-система (`patches/`)

`Гайд v3 · 2026-10-26 · чужой код, наши правила`

Чужой код (TWRP, vold, keymint-обвязка…) правится **только** патчами. Каждый патч хранится **тройкой** - три копии всегда синхронны:

```
patches/files/
├── original/<путь от корня исходников>   # pristine-файл (как в AOSP/OFox)
├── modified/<тот же путь>                # полный файл с нашими правками
├── patches/<тот же путь>.patch           # unified diff original→modified
├── new/                                  # целиком новые файлы (если нужны)
└── source_snapshot.{json,txt}            # историческая запись миграции (не реестр!)
```

## Правила - править так, без исключений

1. **Правится `modified/`, `.patch` регенерируется** `diff -U3` (`--label a/… b/…`, формат как у соседей), живое дерево приводится к снапшоту. Никогда наоборот: `.patch` вручную не пишут (кроме backfill-исключений с обязательным dry-run).
2. **Проверка:** `apply_patches.py --check` - всё должно быть `[OK]` (таргет совпадает со снапшотом `modified`). Применение: `apply_patches.py --apply --root <корень сборки>` (дёргает `build.sh`). Апплаер сам находит файлы через `rglob` - новые тройки **нигде не регистрируются**.
3. **Живое дерево = `modified`.** Если дерево уже пропатчено (нормальное состояние), оригиналы восстанавливаются реверсом существующих патчей, а не копированием из дерева. После правки дерево синкается - следующий прогон скажет `already matches modified snapshot`.
4. **Верификация тройки:** наложить `.patch` на копию `original` → побайтовое равенство с `modified` (`cmp`) + `patch --dry-run`.

<details>
<summary>🧰 Стек инструментов - кто за что</summary>

| Файл | Роль |
|---|---|
| `apply_patches.py` | `--check` / `--apply`, dry-run `patch -p0 --fuzz=3`, детект дрейфа снапшота |
| `patchlib.py` | Библиотека наложения, структурные патчи, bypass-режим |
| `sync_from_bakfiles.py` | Пересборка снапшотов из `.bak` |
| `sync_tree.py` (корень) | Умный `repo sync` с сохранением локального (`-s` сканер, `-d` только дифф, `-c` интерактив, `-f` форс) |

</details>

<details>
<summary>📦 Что уже покрыто (66 троек)</summary>

- TWRP: `data.cpp` (letterbox), `action.cpp` (`cmd:`-torch), `gui.cpp`, `objects.hpp`, `pages.cpp`, `patternpassword.cpp`, темы; `minuitwrp`: `events.cpp` (вибрация; на GS101 - timed-пульс `brightness`, т.к. `activate` в рекавери не пишется), `graphics.cpp`, `graphics_drm.cpp` (cover-панель), `resources.cpp`; `partition*.cpp/hpp`, `twrp-functions.cpp`, `twrpRepacker.cpp`, `install.cpp`, `spl_check.cpp`.
- Система: vold (`Decrypt`, `MetadataCrypt`, `Weaver1`, `FsCrypt` + заголовки), fastbootd (usb_*), keymaster/KeyMint `Android.bp`, `keystore2/globals.rs`, update_engine, `init.cpp`/`util.cpp`, Soong/AIDL-мосты, `BoardConfigSoong.mk`. (Тройки libfstab в дереве нет - старый пример `fstab.cpp/.h` устарел.)
- GUI: проброс `TW_FRAMERATE` (`libguitwrp_defaults.go`), `listbox.cpp` (скролл без переменной).

</details>
