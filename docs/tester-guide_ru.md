# Тестирование тестовых сборок OrangeFox - полный гайд

> [English version](tester-guide.md)

`Гайд v3 · 2026-10-26 · полевой протокол`

Для тестеров **экспериментальных (тестовых) сборок**. Тестовая сборка может не загрузиться, сломать расшифровку или оставить слот незагружаемым - действуй строго по гайду, и путь назад будет всегда.

> Тестовые сборки: **Format Data запрещён, пока не попросили**. Шей **только** раздел, который сказали (обычно `vendor_boot_X`). Никогда не шей `super`, `boot`, `system`, `modem`, bootloader или радио из тестового пакета без прямого указания.

## Содержание

- [0. Подготовка](#0-подготовка)
- [1. Прошивка тестовой сборки](#1-прошивка-тестовой-сборки)
- [2. Первая загрузка - чек-лист](#2-первая-загрузка--чек-лист)
- [3. Снятие логов - сначала flog.sh](#3-снятие-логов--сначала-flogsh)
- [4. Тест расшифровки](#4-тест-расшифровки)
- [5. Как отрисовывается UI](#5-как-отрисовывается-ui-особенно-фолды-и-планшеты)
- [6. Исходы A-D](#6-исходы-ad)
- [7. Шаблон репорта](#7-шаблон-репорта-копипаст)
- [8. Золотые правила / бэкап / шпаргалка](#8-золотые-правила-бэкап-шпаргалка)

## 0. Подготовка

- ПК с рабочими `platform-tools` (`fastboot`, `adb`). Кабель + желательно порт/хаб USB 2.0 (меньше глюков). Батарея **> 50%**.
- **Заводские образы** под твой девайс скачаны и распакованы (нужен минимум заводской `vendor_boot.img` под твой слот) - или сдампь свой заранее ([§8](#8-золотые-правила-бэкап-шпаргалка)).
- Знай коднейм (`shiba`, `comet`, `grizzly`…) и текущий слот: `fastboot getvar current-slot`.
- Сохрани всё важное.

<details>
<summary>✅ Сначала MD5 - Telegram режет файлы</summary>

- Сверь хеш образа **ДО** прошивки. Telegram иногда отдаёт `.img` обрезанными (так реально было при тестировании R11 и стоило дней бессмысленной отладки). Мейнтейнер постит `md5` рядом с каждой тестовой сборкой (или в закрепе https://t.me/OFRPforTensor) - сверяй: `md5sum OrangeFox-test-xxx.img` (Windows: `certutil -hashfile OrangeFox-test-xxx.img MD5`).
- Хеш не совпал - загрузка битая, а **не** баг. Качай заново (лучше из GDrive-папки в канале) и ничего не репорть. Бутлуп с пустым pstore при несовпадении хеша - это сначала битый файл, а не баг.

</details>

## 1. Прошивка тестовой сборки

Мейнтейнер говорит, **какой файл шить и в какой слот**. Дефолтный путь - **AIO-установщик** ниже. Ручные fastboot-потоки - legacy (до-AIO), только по прямому указанию.

<details>
<summary>📦 Шаг 0 - распаковывать или нет</summary>

- Файл выглядит как `OrangeFox-<version>-<tag>-aio.zip` (десятки МБ).
- **Установка с ПК (ниже): РАСПАКОВАТЬ.** Windows: правый клик → **Извлечь всё…**. Linux: `unzip OrangeFox-*.zip -d ofox-install && cd ofox-install`. После распаковки должны лежать: `install-desktop.sh`, `install-desktop.bat`, `install-recovery.sh`, `export.txt`, `module.prop`, `customize.sh`, `bin/`, `META-INF/` и один пейлоад `*.ramdisk.lz4`.
- **Magisk / KernelSU / установка из рекавери: НЕ распаковывать.** Шей `.zip` как есть.
- Ошибка новичков №1: запуск из окна архиватора. Так не работает никогда. На ПК ничего ставить не надо: platform-tools едут внутри (`bin/`).

</details>

<details>
<summary>🖥️ Вариант A - ПК / Linux (из bootloader)</summary>

1. Распакуй zip, открой терминал **в той папке**.
2. Телефон в классический fastboot (bootloader): держи **Громкость вниз + Power** (`FASTBOOT MODE` / `START` - НЕ fastbootd). Или из системы: `adb reboot bootloader`.
3. Подключи USB, запусти `./install-desktop.sh` (или двойной клик по `install-desktop.AppImage`).
4. Ответь на меню-стрелки (слоты - дефолт `both` самый безопасный). Смотри строки: snapshot → backup → rebuild → flash → byte-compare.
5. Согласись на перезагрузку в рекавери. Бэкап ляжет в `backup/<дата-время>/` (+ `install.log`). Успех = `RESULT: OK`.

</details>

<details>
<summary>🪟 Вариант B - ПК / Windows (из bootloader)</summary>

- Как A, но двойной клик по `install-desktop.bat`. Первый раз в жизни: поставь **Google USB driver** (без него `fastboot devices` пуст). Если `.bat` моргнул и закрылся - запусти его из `cmd` в той папке, чтобы прочитать ошибку.

</details>

<details>
<summary>📲 Вариант C - Magisk / KernelSU (загруженный рутованный Android)</summary>

1. НЕ распаковывай. Положи `.zip` на телефон (скачай туда или MTP/`adb push`).
2. Приложение Magisk (или KernelSU) → **Модули** → **Установка из хранилища** → выбери zip → дождись install OK.
3. Модуль **удалит себя сам** - разовый установщик, ничего не остаётся. Бэкап остаётся в `/sdcard/backup_vendor_boot/`.
4. Перезагрузись в рекавери сам (Громкость вниз + Power → **Recovery mode**, или `adb reboot recovery`). Нужно: загруженный Android + рут. Слоты решает `SLOT=` в `export.txt` (дефолт `both`).

</details>

<details>
<summary>🔄 Вариант D - из рекавери (ПК и рут не нужны)</summary>

1. НЕ распаковывай. Положи `.zip` на телефон (MTP/`adb push`/Загрузки).
2. Любое рекавери (TWRP/OrangeFox) → **Install** → выбери zip → свайп. Шить рекавери, в котором сидишь, можно - оно работает из RAM.
3. Смотри `RESULT: OK` + путь бэкапа (`/sdcard/backup_vendor_boot/` или `/tmp/recovery_install/`).
4. **Reboot → Recovery** (не System), чтобы сразу загрузиться в новую сборку.

</details>

<details>
<summary>📼 Legacy: ручная прошивка fastboot (только до-AIO)</summary>

- Дефолт (пример слот A): `fastboot flash vendor_boot_a OrangeFox-test-xxx.img` → `fastboot reboot recovery`.
- **Pixel 6 (gs101) особо - только до-AIO сборки** (`test_1-gs101`): там нет `vendor_kernel_boot`, поэтому шьётся `.ramdisk.lz4` в platform-фрагмент (обрати внимание на двоеточие = пустое имя фрагмента): `fastboot flash vendor_boot_a: OrangeFox-test-xxx-gs101.ramdisk.lz4`. `:default` или `:recovery` НЕВЕРНЫ - первый схлопывает таблицу и убивает стоковый dlkm, второй шьёт не тот фрагмент.
- Для `-aio` сборок **НИКОГДА** не шей raw-cpio через fastboot - образ семейно-нейтрален, ему нужен свап установщика (USB-контроллер, fstab, KeyMint-манифесты). Raw-AIO на Pixel 6 грузится со сломанным USB (нет adb) и неверной геометрией. Используй AIO-установщик выше.

</details>

После любого варианта: дождись главного экрана рекавери, затем иди дальше. Копию `backup/` (ПК) или `/sdcard/backup_vendor_boot/` держи до конца тестов - это откат. Конфиг: `export.txt` (`SLOT=both|a|b|current`, `MIN_FREE_MB=7` строгий). Логи: `backup/<дата-время>/install.log` (ПК) или `/tmp/recovery_install/install.log` (на девайсе).

## 2. Первая загрузка - чек-лист

После `fastboot reboot recovery` дождись главного экрана (USB-стек и сервисы расшифровки стартуют с задержкой), затем:

> **adb на экране пароля (не-Stable тестовые сборки).** Тебе приехала НЕ-Stable (Beta) сборка - она не гасит adb/MTP как Stable. Поэтому adb на начальном PIN-запросе **может работать, а может нет - разработчик честно не знает** (его собственный девайс сейчас незашифрован, проверить этот путь нечем). Твоя работа: зафиксировать факт. `adb devices` видит → гоняй всё прямо там, репорть **присутствует**. `no devices` → уйди с экрана пароля (PIN если тач работает, или кнопка **Skip** / назад в главное меню без расшифровки - на некоторых сборках её нет; тогда только PIN), гоняй после, репорть **отсутствует**. Логи пишутся с самого старта в обоих случаях - снять их позже ничего не теряет.

```bash
adb devices -l
adb shell 'getprop sys.usb.config; getprop init.svc.vendor.keymint.rust-trusty; getprop init.svc.vendor.keymint-trusty; uname -r; getprop ro.boot.slot_suffix'
```

| Проверка | Здоровое значение |
|---|---|
| `adb devices` | девайс в списке как `recovery` |
| `sys.usb.config` | `mtp,adb` |
| сервис KeyMint | `running` (ровно один из rust-trusty / trusty) |
| `uname -r` | ядро, под которое собрано (не уверен - спроси мейнтейнера) |
| суффикс слота | слот, который шил (`_a` / `_b`) |

Дальше - логи, §3, сначала `flog.sh`. Одна команда заменяет все ручные пулы ниже.

## 3. Снятие логов - сначала flog.sh

`flog.sh` едет внутри каждой текущей тестовой сборки. Одна команда собирает всю пачку - **предпочти ручному `adb pull`**: строго больше (ранние AIO-логи, пропсы, состояние подсветки, pstore, логи установщика) + world-readable права, чтобы MTP/ПК-ридеры открывали каждый файл.

Из рекавери - по adb **или** из терминала Fox (Advanced → Terminal, adb вообще не нужен): `adb shell flog.sh`. В конце вывода - место пачки (запомни timestamp): `flog: DONE -> /data/media/0/fox_logs/fox_logs_<device>_<family>_<stamp>.tar.gz` (когда userdata залочена, в DONE вместо tar.gz будет голая директория `/tmp/fox_logs/<stamp>/` - забирай сразу, RAM умрёт при ребуте).

<details>
<summary>💾 Куда ляжет пачка - матрица userdata</summary>

| Состояние userdata | Назначение | Как забрать на ПК |
|---|---|---|
| Расшифрована + пишется (PIN введён или шифрования нет) | `/data/media/0/fox_logs/<stamp>/` - **переживает ребут** | `adb pull` когда угодно позже (даже из загруженной системы) или скопировать по MTP |
| Залочена / не пишется | `/tmp/fox_logs/<stamp>/` - RAM, **умрёт при ребуте** | `adb pull` **сразу**, пока в рекавери |

</details>

<details>
<summary>📋 Что внутри пачки</summary>

| Файл | Источник | Доказывает |
|---|---|---|
| `recovery.log`, `weaver.log` | `/tmp/*.log` | поток рекавери + расшифровки |
| `reflash_twrp.log`, `install-*.log` | установка на девайсе | путь reflash/install (если использовался) |
| `aio_stub.log`, `aio_runatinit.log` | стаб + Rust early-boot | детект семьи, свап, USB-rescue |
| `dmesg.log`, `logcat.log` | ядро + система | железо тача/экрана/USB/расшифровки |
| `props.txt` | `getprop` + ключевые пропсы | слот, семья, сервисы, состояние USB |
| `backlight.txt`, снапшот батареи+health | sysfs подсветки, health HAL | проблемы экрана + питания в сломанном виде |
| `pstore*` | `/sys/fs/pstore/` | краши ядра (переживают ребут) |
| `families.txt`, `pixelrunatboot.json` | конфиг установщика | какой пейлоад/слот-политика дали эту загрузку |

Строки `- missing:` - тоже ответ (нет `aio_stub.log` = стаб не логгировал). Не удаляй их; вывод консоли тоже шли. Для железного дебага (тач, экран, USB, расшифровка, сенсоры) после `recovery.log` важнее всего `dmesg`/`logcat` - всегда полные файлы, никогда выдержки.

</details>

<details>
<summary>🆘 Нет adb вообще (мёртвый USB) / ручной фолбэк</summary>

- Нет adb: терминал Fox → `flog.sh` → пачка ляжет на userdata если расшифрована → ребут в систему, забрать по `adb`/MTP. Если userdata тоже залочена - фотографируй ключевые экраны, иди к исходу B/C.
- Ручной фолбэк (сборки без `flog.sh`):

```bash
adb pull /tmp/recovery.log
adb pull /tmp/weaver.log
adb shell 'dmesg > /tmp/dmesg.log; logcat -d -v time > /tmp/logcat.log'
adb pull /tmp/dmesg.log
adb pull /tmp/logcat.log
```

</details>

## 4. Тест расшифровки

1. Если девайс просит PIN/пароль/паттерн - введи.
2. Успех - видишь свои файлы (Internal Storage), а в `recovery.log` есть `User 0 Decrypted`.
3. Тач не работает: это **НЕ провал теста**, пока мейнтейнер не сказал иначе. Репорть, продолжай по `adb`.
4. Пришли обратно: пачку `flog.sh` (§3) + видна ли `/data/media`.

## 5. Как отрисовывается UI - особенно фолды и планшеты!

1. **Какой экран показывает UI** (фолд: cover/внешний или inner? Сложен или разложен при загрузке?).
2. **Чёрные бары**: есть? С каких сторон? Бары - нормальный letterbox-режим, важны факт + симметрия.
3. **Растягивание**: круги - круги или эллипсы? Текст/иконки в масштабе рамок или «плавают»? Клавиатура во всю ширину или уже экрана?
4. **Поворот**: UI прямо или на 90°? (Внутренний экран фолда может поворачиваться - куда?)
5. **Попадание тача**: тапы там, где показано? (Фолд: внешний тач работает на внешнем, а не наоборот?)
6. **Сфотографируй экран** другим телефоном - фото бьёт любые слова. Фолд: фото сложен + разложен.

Если есть adb, добавь: `adb shell 'getprop DOF_SCREEN_W; getprop DOF_SCREEN_H; getprop DOF_PROGRESSIVE_SCALE'`.

<details>
<summary>🧪 Фич-тесты (только если попросили)</summary>

- **Тач**: работает / частично / мёртв + `dmesg | grep -i touch`. **Фонарик / вибро**: работает или нет. **OTG / USB-мышь**: работает или нет - с воткнутым диском и отключённым ПК гони `otg_test.sh` (терминал Fox) ради вердиктов T1-T8, приложи `otg_test_<device>_<family>_<stamp>.tar.gz`.
- **Format Data / Reflash Recovery на тестовых сборках ЗАПРЕЩЕНЫ**, пока мейнтейнер явно не разрешил (некоторые сборки отказываются в reflash по дизайну - текст отказа репорть, он подтверждает работу гарда).

</details>

## 6. Исходы A-D

**Исход A - грузится, adb работает:** делай §2-5, шли репорт (§7). Готово.

<details>
<summary>🅱️ Исход B - грузится, но НЕТ adb</summary>

1. Жди полные 60 с, передёрни кабель, попробуй другой порт / хаб USB 2.0.
2. Затем логи без adb: терминал Fox → `flog.sh` (§3).
3. Проверь fastbootd: `fastboot devices`. Виден там = ядро живо, проблема в USB-гаджете - репорть.
4. Полная тишина → иди к исходу C (откат).

</details>

<details>
<summary>🅲 Исход C - НЕ грузится (лого / чёрный / бутлуп)</summary>

**Не паникуй. Не шей случайное.** Откат:

1. Прошей **заводской (known-good)** `vendor_boot.img` в **тот же слот**: `fastboot flash vendor_boot_a stock_vendor_boot.img` (или `_b`).
2. Загрузись в систему штатно.
3. С рутом сними дампы крашей ядра **до перезаписи** (второй цикл загрузки может их ротировать - быстро, одна загрузка):

```bash
su
ls /sys/fs/pstore/
cat /sys/fs/pstore/console-ramoops-0 > /sdcard/ramoops_console.txt
cat /sys/fs/pstore/dmesg-ramoops-0  > /sdcard/ramoops_dmesg.txt
dmesg > /sdcard/dmesg_boot.txt
getprop ro.boot.slot_suffix; uname -r
```

4. Стащи на ПК (`adb pull /sdcard/ramoops_console.txt` …), пришли **все** + какой файл шился в какой слот.

**Трюк boot → crash → switch-slot:** если слот B с тестовой сборкой бутлупится, bootloader может сам пометить его незагружаемым и откатиться на слот A - проверь `fastboot getvar current-slot` после фейла. Или форсируй другой слот (`fastboot --set-active=a` / `b`), загрузись в систему, сними pstore как выше. Ключевое: **pstore переживает ребут, но немного** - снимай на ПЕРВОЙ успешной загрузке после краша, дважды не ребут.

</details>

<details>
<summary>🅳 Исход D - грузится с adb, но расшифровка ПАДАЕТ</summary>

Шли пачку `flog.sh` (§3) плюс:

1. Ручной прогон HAL (~15 с, затем Ctrl-C): `adb shell '/vendor/bin/hw/android.hardware.security.keymint-service.rust.trusty --dev /dev/trusty-ipc-dev0'` (для C++ HAL сборок мейнтейнер даст другой путь).
2. Состояния сервисов: `adb shell 'getprop init.svc.recovery_storageproxyd; getprop init.svc.recovery_weaver'`.

</details>

## 7. Шаблон репорта (копипаст)

```
Файл сборки : <точное имя файла>
Прошит в    : vendor_boot_a / vendor_boot_b
Девайс      : <коднейм, напр. grizzly>
Исход       : грузится+adb / грузится-без-adb / не-грузится / грузится-без-расшифровки
adb виден   : да / нет
adb на пароле: есть / нет (если девайс зашифрован)
расшифровка : ок / провал / не пробовал
тач         : ок / мёртв / частично
UI          : бары (какие стороны) / растянут / повёрнут / какой экран (фолд: сложен/разложен) + фото
Приложено   : fox_logs/<stamp>/ (вся пачка flog.sh: recovery.log, weaver.log,
              dmesg.log, logcat.log, aio_*.log, props.txt, …)
              - или те же файлы вручную
              (+ ramoops_console.txt, ramoops_dmesg.txt, dmesg_boot.txt если исход C)
Заметки     : <что-то необычное: сколько грузилось, тексты ошибок, ...>
```

## 8. Золотые правила, бэкап, шпаргалка

<details>
<summary>🏆 Золотые правила</summary>

1. Одно изменение за раз. Никогда не мешай тестовую сборку с другими модами.
2. Заводские (known-good) образы готовы **до** прошивки, а не после.
3. Сначала логи, выводы потом - всегда прикладывай полные файлы.
4. НЕ жми «Reflash Recovery», НЕ делай Format Data, НЕ шей другие разделы на тестовых сборках, пока не сказали.
5. Что-то повело себя неожиданно - сфотографируй/запиши точный текст. «Не заработало» - не репорт.

</details>

<details>
<summary>💾 Как сдампить заводской vendor_boot (бэкап до тестов)</summary>

Нет заводских образов под рукой? Сдампь known-good `vendor_boot` прямо с девайса **до** прошивки тестовой сборки. Проще всего - bootloader, рут не нужен (работает и для Pixel 6):

```bash
fastboot fetch vendor_boot_a ./vendor_boot_a.stock.img
fastboot fetch vendor_boot_b ./vendor_boot_b.stock.img
```

Из рутованной системы (оба слота сразу):

```bash
su
dd if=/dev/block/by-name/vendor_boot_a of=/sdcard/vendor_boot_a.stock.img bs=4M
dd if=/dev/block/by-name/vendor_boot_b of=/sdcard/vendor_boot_b.stock.img bs=4M
```

Из рекавери (adb, там рут по умолчанию):

```bash
adb shell 'dd if=/dev/block/by-name/vendor_boot_a of=/sdcard/vendor_boot_a.stock.img bs=4M'
adb shell 'dd if=/dev/block/by-name/vendor_boot_b of=/sdcard/vendor_boot_b.stock.img bs=4M'
adb pull /sdcard/vendor_boot_a.stock.img
adb pull /sdcard/vendor_boot_b.stock.img
```

Контроль: размер обязан быть **67108864 байта (64 МБ)**. Прошить обратно: `fastboot flash vendor_boot_a vendor_boot_a.stock.img` (или `_b`).

</details>

<details>
<summary>⌨️ Шпаргалка fastboot/adb</summary>

```bash
fastboot getvar current-slot
fastboot --set-active=a|b
fastboot flash vendor_boot_a|b <файл>   # legacy ручной поток (до-AIO)
fastboot flash vendor_boot_a: <ramdisk.lz4>   # legacy, только Pixel 6 (обрати внимание на двоеточие)
./install-desktop.sh                              # AIO-установщик, Linux (из bootloader)
install-desktop.bat                               # AIO-установщик, Windows (из bootloader)
fastboot fetch vendor_boot_a|b ./backup.img   # бэкап без рута
fastboot reboot recovery
fastboot reboot
adb devices -l
adb shell flog.sh                                 # полная пачка репорта (предпочтительно)
adb pull /data/media/0/fox_logs/<stamp>          # забрать пачку (userdata)
adb pull /tmp/fox_logs/<stamp>                   # забрать пачку (нет userdata, срочно)
adb pull /tmp/recovery.log
adb pull /tmp/weaver.log
adb shell 'dmesg > /tmp/dmesg.log; logcat -d -v time > /tmp/logcat.log'
adb pull /tmp/dmesg.log
adb pull /tmp/logcat.log
adb shell 'dmesg | grep -i -E "touch|keymint|trusty|gsc|sg1" | head -30'
```

</details>
