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
5. `build.sh -n <tag>` → в логе `[PIXELCFG] merged N devices`,
   `Entries packed`. Прошивка обоих слотов через установщик → проверки
   из `diagnostics_ru.md`.

## Новый SoC (семья `families/<fam>/`)

| Файл | Содержимое |
|---|---|
| `family.conf` | `FAMILY`, `UFS_ADDR`, `USBCTRL` (если не `11210000.dwc3`), `USBBUS` (если контроллер под `simple_usb_bus`) |
| `family.json` | То же + `keymint` (rust\|cpp), общие `props` |
| `recovery.fstab` + `recovery.wipe` + `twrp.flags` | Swap-kit файлы: едут как `*.<fam>`, установщик выбирает после распаковки |
| `recovery/` | Family-оверлей рамдиска (rc-стабы) |
| `etc/` | VINTF-фрагменты (keymint-манифест schema 2.0) |

Дальше: стоковый `vendor_boot` девайса → сверить UFS/USB-адреса с
`twrp.flags` → `board-info.txt` → тест по `tester-guide.ru.md`.
Kernel-профилей нет (стоковое ядро сохраняется — см.
`kernel-profiles_ru.md`), `family.mk` нет (фрагмент сборки только у
`families/aio`).

## Матрица семейств (факты для конфигов)

| Семья | UFS | DWC3 USB | KeyMint |
|---|---|---|---|---|
| gs201 | `14700000` | `11210000.dwc3` | cpp |
| zuma | `13200000` | `11210000.dwc3` | rust |
| zumapro | `13200000` | `11210000.dwc3` | rust |
| laguna | `3c400000` | `c400000.dwc3` | rust |
| malibu | `3c2d0000` | `a210000.dwc3` | rust |
| gs101 | `14700000` | `11110000.dwc3` | cpp |

Все девайсы едут в едином универсальном пейлоаде; семейные файлы
выбираются при установке/загрузке (swap-kit + стаб + движок).

## gs101 (Tensor G1, Pixel 6 series) — заметки по железу

У gs101 нет раздела `vendor_kernel_boot`: стоковый `vendor_boot` несёт
фрагменты platform + dlkm + dtb. Универсальный пейлоад — это
platform-фрагмент (first-stage + recovery слиты), прошивка идёт
хирургией фрагментов — `fastboot flash vendor_boot: <ramdisk>` (пустое
имя = platform-фрагмент; НЕ `:default` — тот схлопывает всю секцию в
одну запись и убивает стоковый dlkm). Установщик сам стянет текущий
`vendor_boot` с девайса, заменит только platform-запись и зашьёт
назад: стоковые dlkm+dtb выживают, first-stage продолжает грузить
стоковые модули из dlkm автоматом.

Ручная прошивка (то, что автоматизирует установщик):

```bash
fastboot flash vendor_boot: OrangeFox-R12.0-test_x-aio.ramdisk.lz4
fastboot reboot recovery
```

First-stage (`vendor_ramdisk/` стейджинг: `/init`, линкер, sepolicy,
fstab) колбэк не пакует никогда — только читает для списков файлов,
так что кластер его не трогает по построению.
