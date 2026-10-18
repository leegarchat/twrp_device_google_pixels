> [English version](diagnostics.en.md)
# Диагностика на девайсе

adb в рекавери — root по умолчанию (на Stable-сборках adbd остановлен
by design — см. `build-system_ru.md`; для отладки брать Beta).

## Базовый опрос (первые 30 секунд)

```bash
adb shell 'getprop ro.hardware; uname -r; getconf PAGESIZE'
adb shell 'grep -iE "ko_loader|susfs_fix|otg_patch|pixelrunatboot" /tmp/recovery.log'
adb shell 'ls /dev/ko_stage/*/ | head; cat /proc/modules | grep -cE "touch|goodix"'
adb shell 'dmesg | grep -iE "LGZ|OFOX" | head'
```

## Карта симптомов

| Симптом | Куда смотреть |
|---|---|
| Не грузится вообще | `dmesg` с ПК (fastboot), слот (`bootctl`/`slot-detect`), оба ли слота прошиты |
| Висит до GUI | `/tmp/recovery.log`: `boot`-стадия (модули), `servicemanager.ready`, keymint-статус |
| Нет тача | `lsmod` (тач-модули из `touch_modules`?), `/dev/input/`, depth-guard в `ko-fetch` |
| Не расшифровывает | Статус keymint/weaver-сервисов, `.weaver`-слот, `ro.crypto.fs_crypto_blkdev` |
| Нет USB/OTG | `vbus_paths`, `patch_dwc3`, UDC-привязка в логе `otg-auto` |
| GUI тормозит | `TW_FRAMERATE` в сборочном логе (`-DTW_FRAMERATE=` в cflags) |
| Фолд: картинка/тач на разных экранах | `DOF_SCREEN_W/H`, выбор DSI-коннектора (cover-патч) |
| Format Data падает | Строки `FoxFstab`/`FoxTeardown`/`FoxFormat` в `/tmp/recovery.log` (см. ниже) |

## Format Data на мультидевайсной /data

На аппаратах, чей стоковый `fstab` декларирует сет девайсов `/data`
(`device=zoned|exp|exp_alias`, семьи malibu/laguna), рекавери не строит f2fs —
повторяет стоковый factory reset. Живая геометрия парсится в момент Format
из `/first_stage_ramdisk/system/etc/fstab.<family>` загруженного слота
(вендорные DFE-правки и наши снапшоты обходятся); отсутствующие ноды
скипаются, а без exp-участников (пустой сет или одинокий zoned) легаси-путь
однодевайсных семей остаётся нетронутым.

Стадии в `/tmp/recovery.log`:
1. `FoxFstab: parsing ...` + `/data set: N device(s)` — что нашлось;
2. `FoxTeardown: unmounted ...` + `removing dm mapping ...` — обратное снятие
   расшифрованного стека до `Unmap_Super_Devices` (ошибка `still in use`
   означает, что что-то держит `/data`: закрыть, повторить; до этого момента
   ничего не уничтожается);
3. `FoxFormat: erasing /data...` — discard userdata с проверкой blank,
   затем вайп `/metadata`, сброс memtag, вайп `trusty_userdata`, misc-флаги.
   `Can_Be_Mounted=false`: vold форматирует сет при следующей загрузке
   Android (в успешном логе — «Android sets up /data on its next boot»).

`EINVAL`/`EBUSY` на `removing dynamic partition: userdata_exp.*` = сборка
без teardown (сначала обновиться, потом репортить).

## Полный протокол для тестеров

[`tester-guide.ru.md`](tester-guide.ru.md) — установка, исходы A–D,
pstore (`console-ramoops`/`dmesg-ramoops`), дамп `vendor_boot`,
UI-проверки фолдов/планшета. Репорты без логов не чинятся.
