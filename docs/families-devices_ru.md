> [English version](families-devices.md)

# Добавление девайса или нового SoC

`Гайд v3 · 2026-10-26 · без правок .mk`

Добавление девайса не требует правок `.mk`: `devices/*` подхватываются вилдкардами (`TARGET_RECOVERY_DEVICE_DIRS`, мердж конфига).

## Новый девайс - 5 шагов

1. `devices/<codename>/device.conf`: `DEVICE=<codename>`, `FAMILY=<fam>`.
2. `devices/<codename>/pixel.json` - секция по схеме из `device-config_ru.md`. `touch_modules` сверить со стоковым `vendor_dlkm` девайса; для фолда добавить `is_fold` + геометрии.
3. `devices/<codename>/recovery/root/init.recovery.<codename>.rc` (обычно один `import` общего rc).
4. Опционально `devices/<codename>/twrp.flags` - поверх family-файла (сборщик положит как `<device>.twrp.flags`, рантайм подменит после резолва девайса).
5. `build.sh -n <tag>` → в логе `[PIXELCFG] merged N devices`, `Entries packed`. Прошивка обоих слотов через установщик → проверки из `diagnostics_ru.md`.

<details>
<summary>🏭 Новый SoC - семейство `families/<fam>/`</summary>

| Файл | Содержимое |
|---|---|
| `family.conf` | `FAMILY`, `UFS_ADDR`, `USBCTRL` (если не `11210000.dwc3`), `USBBUS` (если контроллер под `simple_usb_bus`) |
| `family.json` | То же + `keymint` (rust\|cpp), общие `props` |
| `recovery.fstab` + `recovery.wipe` + `twrp.flags` | Swap-kit файлы: едут как `*.<fam>`, установщик выбирает после распаковки |
| `recovery/` | Family-оверлей рамдиска (rc-стабы) |
| `etc/` | VINTF-фрагменты (KeyMint-манифест schema 2.0) |

Дальше: стоковый `vendor_boot` девайса → сверить адреса UFS/USB с `twrp.flags` → `board-info.txt` → тесты по `tester-guide_ru.md`. Kernel-профилей нет (стоковое ядро сохраняется - см. `kernel-profiles_ru.md`), `family.mk` нет (build-фрагмент только у `families/aio`).

</details>

## Матрица семейств - факты для конфигов

| Семейство | UFS | DWC3 USB | KeyMint |
|---|---|---|---|
| gs201 | `14700000` | `11210000.dwc3` | cpp |
| zuma | `13200000` | `11210000.dwc3` | rust |
| zumapro | `13200000` | `11210000.dwc3` | rust |
| laguna | `3c400000` | `c400000.dwc3` | rust |
| malibu | `3c2d0000` | `a210000.dwc3` | rust |
| gs101 | `14700000` | `11110000.dwc3` | cpp |

Каждый девайс едет в едином универсальном пейлоаде; семейные файлы выбираются при установке/загрузке (swap-kit + стаб + движок).

<details>
<summary>📼 gs101 (Tensor G1, Pixel 6) - железные заметки</summary>

- У gs101 **нет** раздела `vendor_kernel_boot`: стоковый `vendor_boot` несёт фрагменты platform + dlkm + dtb. Универсальный пейлоад - platform-фрагмент (first-stage + recovery слиты), шьётся прямой записью во фрагмент - `fastboot flash vendor_boot: <ramdisk>` (пустое имя = platform-фрагмент; **не** `:default` - тот схлопывает всю секцию в одну запись и убивает стоковый dlkm).
- Установщик сам забирает `vendor_boot` с девайса, меняет только platform-запись и шьёт обратно: стоковые dlkm+dtb выживают, first-stage продолжает автокормить стоковые модули из dlkm.
- Ручная прошивка (то, что автоматизирует установщик):

```bash
fastboot flash vendor_boot: OrangeFox-R12.0-test_x-aio.ramdisk.lz4
fastboot reboot recovery
```

</details>
