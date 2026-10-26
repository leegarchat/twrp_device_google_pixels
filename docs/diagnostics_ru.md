> [English version](diagnostics.md)

# Диагностика на девайсе

`Гайд v3 · 2026-10-26 · что смотреть, если не грузится`

adb в рекавери - root по умолчанию. На сборках **Stable** adbd остановлен by design (см. `build-system_ru.md`) - для отладки брать **Beta**.

## Базовый опрос - первые 30 секунд

```bash
adb shell 'getprop ro.hardware; uname -r; getconf PAGESIZE'
adb shell 'grep -iE "ko_loader|susfs_fix|otg_patch|pixelrunatboot" /tmp/recovery.log'
adb shell 'ls /dev/ko_stage/*/ | head; cat /proc/modules | grep -cE "touch|goodix"'
adb shell 'dmesg | grep -iE "LGZ|OFOX" | head'
```

<details>
<summary>🗺️ Карта симптомов - куда смотреть</summary>

| Симптом | Куда смотреть |
|---|---|
| Не грузится вообще | `dmesg` с ПК (fastboot), слот (`bootctl`/`slot-detect`), оба ли слота прошиты |
| Висит до GUI | `/tmp/recovery.log`: стадия `boot` (модули), `servicemanager.ready`, статус KeyMint |
| Нет тача | `lsmod` (модули тача из `touch_modules`?), `/dev/input/`, depth guard в `ko-fetch`. Полный промах на модульном ядре поднимает `fox.force_hw_gui=1` (клавиши VOL+/VOL-/POWER); на монолите/частичном промах терпится - какое: строки `is_monolith_cached` в `/tmp/recovery.log` (нет `/proc/modules` → настоящий монолит; пустой lsmod → монолит; канарейка `empty_checker_module` без следа `/proc/empty_check_gki` → частичный монолит, Sultan-стайл: поддельные RC=0 и пустой lsmod) |
| Не расшифровывает | Статус сервисов KeyMint/weaver, слот `.weaver`, `ro.crypto.fs_crypto_blkdev` |
| Нет USB/OTG | Пути `vbus_paths`, `patch_dwc3`, бинд UDC в логе `otg_auto`. Блок гейта (ядро новее прошивки) = только девайс-режим, adb цел. Диск воткнут, но не монтируется: гони `otg_test.sh` на девайсе (ПК отключён) - проверки T1-T8 (диск, раздел, демон, линк `/dev/block/otg-usb`, match uevent, тома, mount, dedup) |
| Путаница dm | `/tmp/fox_dm_state` (`<name> <dm-target>`, `# v1 epoch=`): единственная рантайм-правда. `dm-watch` снапшот-only - не воскресит узел, который формат только что снёс |
| Нет % батареи | Жив ли health HAL (`android.hardware.health@2.1-service`)? Снапшот батареи в `flog.sh` |
| Тормозит GUI | `TW_FRAMERATE` в логе сборки (`-DTW_FRAMERATE=` во флагах) |
| Фолд: картинка/тач на разных экранах | `DOF_SCREEN_W/H`, выбор DSI-коннектора (cover-патч) |
| Format Data падает | Строки `FoxFstab` / `FoxTeardown` / `FoxFormat` в `/tmp/recovery.log` (ниже) |

</details>

<details>
<summary>💾 Format Data на мультидевайс `/data` - два маршрута</summary>

На аппаратах, чей стоковый `fstab` объявляет набор устройств `/data` (`device=zoned|exp|exp_alias`, семейства malibu/laguna), рекавери зеркалит стоковый factory reset. Живая геометрия парсится в момент формата из `/first_stage_ramdisk/system/etc/fstab.<family>` загруженного слота (вендорские DFE-правки и наши снапшоты обходятся); отсутствующие узлы скипаются, а без exp-участников (пустой набор или одинокий zoned-компаньон) остаётся нетронутым legacy-путь одного девайса. Несканируемый fstab или объявленный exp-набор с нулём выживших узлов - громкий отказ от формата (fail closed) вместо угадывания.

Стадии в `/tmp/recovery.log`:

1. `FoxFstab: parsing ...` + `/data set: N device(s)` - что найдено;
2. quiesce: MTP off + `FoxTeardown: unmounted ...` (обратимые размонтирования);
3. merge-проверка (Unmap по размонтированному стеку; sweep теперь скипает все crypto dm-узлы), затем решение о маршруте на живом стеке;
4. расшифровано (живой dm-стек) = маршрут **(a-dm)**, как legacy-поток: `mkfs` через живой crypto dm (vold-точные аргументы, mapper-пути + `@alias`) + тут же mount-verify; ключи живы, `/data` пригодна в сессии, без вайпа метадаты, без нюка Титана. Залочено/DFE (стека нет) = маршрут **(b)**: снос dm сверху вниз (`FoxTeardown: removing dm mapping ...`; дальше перезагрузка пересоздаст стек через decrypt, ключи всё ещё в `/metadata`), discard userdata с верификацией header-kill, подивайсные дискарды компаньонов (zoned отдаётся vold-провижинингу).
5. **Фейл (a-dm) падает в (b)** внутри того же вызова. Хвост только на (b): вайп `/metadata`, сброс memtag, нюк Титана M (`titan-wipe`), вайп `trusty_userdata`, misc-флаги; `Can_Be_Mounted=false` только на (b). Авторитетным остаётся vold-реформат следующей загрузки через свежий dm-стек.
6. Header-проверка доказывает только убийство 4K-заголовка, никогда весь вайп; крипто-стирание на маршруте (b) полно через вайп `/metadata` + нюк Титана.

`EBUSY` на teardown = что-то держит `/data`: чисто перезагрузите рекавери и повторите; до этой точки ничего не разрушено. `EINVAL`/`EBUSY` на `removing dynamic partition: userdata_exp.*` = сборка без teardown (обновитесь, затем репортьте).

</details>

<details>
<summary>📝 DFE / plaintext и legacy-правила</summary>

- DFE/plaintext-аппараты (нет живого crypto-стека) держат legacy rm-rf-маршруты - разбиндить нечего. Голый `mkfs` на мультидевайс-наборе не поддерживается нигде: для вайпа/рестора `/data` тикайте чекбокс rm-rf, никогда plain-format.
- Правило порядка (legacy `/data`): rm-rf-стиль вайпает живой маунт **до** любого размонтирования; mkfs-стиль сначала размонтирует. Размонтирование до rm-rf-вайпа ломает перемонт - протухший crypto-мэппер всё ещё держит raw-девайс (`EBUSY`), либо сырой шифртекст сам по себе не маунтится (`EINVAL`).
- Отмеченный rm-rf на `/data` = ПОЛНЫЙ файловый вайп (тот же результат, что mkfs, другим методом, включая internal storage): обходит media-preserving ветку, которая иначе сохранила бы `/data/media` и - через скип `OF_WORKAROUND_BACKUP_BUG` - `/data/data`. Без галки - сохраняющее поведение (factory-reset style).
- Честный ярлык: этот Format Data - **крипто-стирание** (убивает ключи в `/metadata` + Титан, дискардит набор). Физического зануления каждого UFS-блока не гарантирует.

</details>

## Застряли? Сначала логи

В рекавери `flog.sh` (или `adb shell flog.sh`). Один архив `fox_logs_<device>_<family>_<stamp>.tar.gz`, приложите его + модель. Без логов чинить нечего - репорт без архива рассматривать не будут. Полный полевой протокол (исходы A-D, pstore, дамп `vendor_boot`, UI-чеки фолдов/планшета): [`tester-guide_ru.md`](tester-guide_ru.md).
