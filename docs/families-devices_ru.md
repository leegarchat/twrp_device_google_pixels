> [English version](families-devices.en.md)
# Добавление девайса или нового SoC

Добавление девайса не требует правок `.mk`: `devices/*` подхватываются
вилдкардами (`TARGET_RECOVERY_DEVICE_DIRS`, мердж конфига).

## Новый девайс (5 шагов)

1. `devices/<codename>/device.conf`:
   `DEVICE=<codename>`, `FAMILY=<fam>`.
2. `devices/<codename>/pixel.json` — секция по схеме из
   `device-config_ru.md`. `touch_modules` сверить со стоковым
   `vendor_dlkm` девайса; для фолда добавить `is_fold` + геометрии.
3. `devices/<codename>/recovery/root/init.recovery.<codename>.rc`
   (обычно один `import` общего rc).
4. Опционально `devices/<codename>/twrp.flags` — поверх family-файла
   (сборщик положит как `<device>.twrp.flags`, рантайм подменит после
   резолва девайса).
5. `build.sh -f <codename>` → в логе `[PIXELCFG] merged N devices`,
   `Entries packed`. Прошивка обоих слотов → проверки из
   `diagnostics_ru.md`.

## Новый SoC (семья `families/<fam>/`)

| Файл | Содержимое |
|---|---|
| `family.conf` | `FAMILY`, `UFS_ADDR`, `EARLYCON_ADDR`, `USBCTRL` (если не `11210000.dwc3`) |
| `family.json` | То же + `keymint` (rust\|cpp), общие `props`, `default_kernel`, `kernels`-профили (см. `kernel-profiles_ru.md`) |
| `family.mk` | SoC-фрагмент сборки (без cmdline!) |
| `fstab/` + `recovery.fstab` | Пре-рендеренные fstab'ы |
| `recovery/` | Family-оверлей рамдиска (rc-стабы) |
| `twrp.flags` | Флаги семьи (UFS-пути и т.д.) |
| `etc/` | VINTF-фрагменты (keymint-манифест schema 2.0) |

Дальше: стоковый `vendor_boot` девайса → снять cmdline побайтово →
профиль в `kernels` → `board-info.txt` → тест по `tester-guide.ru.md`.

## Матрица семейств (факты для конфигов)

| Семья | UFS | DWC3 USB | KeyMint | Ядра |
|---|---|---|---|---|
| gs201 | `14700000` | `11210000.dwc3` | cpp | 6.1, 6.12 |
| zuma | `13200000` | `11210000.dwc3` | rust | 6.1, 6.12 |
| zumapro | `13200000` | `11210000.dwc3` | rust | 6.1, 6.12 |
| laguna | `3c400000` | `c400000.dwc3` | rust | 6.12 |
| malibu | `3c2d0000` | `a210000.dwc3` | rust | 6.12 |
| gs101 | `14700000` | `11110000.dwc3` | cpp | 6.1 |

Девайсы сгруппированы в один образ на семью (оверрайды `kernels`
отключены через `_kernels_disabled`; возврат — переименовать ключ).

## gs101 (Tensor G1, Pixel 6 series) — особая сборка

У gs101 нет раздела `vendor_kernel_boot`: стоковый `vendor_boot` несёт
фрагменты platform + dlkm + dtb. Наш образ собирается как **один**
platform-фрагмент (first-stage + recovery слиты: в `BoardConfig.mk`
для gs101 выключен `BOARD_INCLUDE_RECOVERY_RAMDISK_IN_VENDOR_BOOT`,
`BOARD_MOVE_RECOVERY_RESOURCES_TO_VENDOR_BOOT` остаётся включённым —
build/make сам подмешивает `TARGET_RECOVERY_ROOT_OUT` в platform),
без dtb и без dlkm-фрагмента.

Стоковые модули ядра (204 `.ko` + `modules.*` из LOS 6.1.145, собрать
из исходников их негде) лежат в `families/gs101/modules/` и копируются
`family.mk` в `/lib/modules/` platform-рамдиска — тем же путём, что
стоковый dlkm-фрагмент. First-stage поднимает UFS один в один как сток.

Тестеры шьют **не образ**, а platform-рамдиск: `build.sh` после сборки
распаковывает `OrangeFox-*-gs101.img` (magiskboot) и кладёт рядом
`OrangeFox-*-gs101.ramdisk.lz4` (стоковый формат `lz4_legacy`,
проверен побайтово против наших грузящихся образов). Прошивка:

```bash
fastboot flash vendor_boot:default OrangeFox-R12.0-test_x-gs101.ramdisk.lz4
fastboot reboot recovery
```

Хост-fastboot сам стянет текущий `vendor_boot` с девайса, заменит
default-фрагмент и зашьёт назад — dtb и bootloader девайса не трогаем.
`reflash_twrp.sh` и ramdisk-снапшот под эту схему ещё не обновлены —
только после подтверждения запуска.
