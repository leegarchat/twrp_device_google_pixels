> [English version](kernel-profiles.en.md)

# Kernel-профили (`kernels` в JSON + `-k`)

`VENDOR_CMDLINE` и `BOARD_BOOTCONFIG` живут не в `.mk`, а в данных:
`families/<fam>/family.json`, секция `kernels` (ключ — версия: `6.1`, `6.12`).
Так cmdline версионируется вместе со стоком и проверяется побайтово.

## Схема профиля

```json
"kernels": {
  "6.12": {
    "cmdline": ["k=v", "..."],
    "bootconfig_append": ["..."]
  }
}
```

Конверт `cmdline` типизирован: `str` — одна строка, `arr` — `["k=v", …]`,
`list` — `[{k: v}, …]`. `bootconfig_append` — список добавок в bootconfig.
`prebuilt`/`ko` — резерв под следующие шаги.

Оверрайд девайса (`devices/<dev>/pixel.json` → `kernels[VER]`) заменяет
семейный профиль **целиком**, а не патчит пофлагово. Сейчас все
оверрайды отключены переименованием ключа в `_kernels_disabled`
(контент на месте для отката) — см. `build-system_ru.md`.

## Механика резолва

```
build.sh -k VER
  → gen_kernel_mk.py --fingerprint FAM VER   # группы device/hash/source
  → families/<fam>/.gen_kernel.mk            # gitignore, до lunch!
  → -include в конце BoardConfig.mk          # VENDOR_CMDLINE, BOARD_BOOTCONFIG, FOX_KERNEL_VER
  → ... сборка ...
  → файл удаляется после сборки
```

- Пре-генерация обязана быть до lunch: `dumpvars` парсит BoardConfig
  прямо во время lunch, позже cmdline уже не подхватится.
- Между kernel-группами — чистка `PRODUCT_OUT`, иначе артефакты смешаются.
- Пустой `VENDOR_CMDLINE` = `$(error)` в BoardConfig: лучше громкий
  фейл, чем незагружаемый образ.
- `kernel_bootcfg` в образе собирается из `.gen_kernel.mk`
  (`FOX_KERNEL_VER` штампуется в reflash-запись) — cmdline образа и
  reflash всегда из одного источника.

## Команды

```bash
./gen_kernel_mk.py --list zuma                  # версии + (default: …)
./gen_kernel_mk.py --fingerprint zuma 6.12      # группы: device/hash/source
./gen_kernel_mk.py --generate zuma shiba 6.12 out.mk  # профиль в файл (сверка со стоком)
```

`default_kernel` в `family.json` — версия без `-k` в интерактиве.
Эталонные cmdline снимаются со стоковых `vendor_boot` (Beta5: ядра 6.1,
Beta4: 6.12; cmdline Beta5 = Beta4 + `binder.impl=rust`).
