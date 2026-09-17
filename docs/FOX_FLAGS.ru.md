# Флаги сборки FOX/OF/TW — что делают и к чему приводят

Справочник по `device/google/pixels` (OrangeFox R12, дерево Android 14).
Все пути ниже — относительно корня Android-репозитория, если не сказано иное.
`pixels/` = `device/google/pixels`.

> Поправка: Stable **останавливает** `adbd` на старте
> (`bootable/recovery/twrp.cpp:246-249`, `ctl.stop adbd`), а включает обратно
> только при входе на страницу `main` (`main.xml:48-65`). Более ранние
> утверждения, что Stable не трогает `adbd`, были неверны.

---

## 1. Ветки сборки: Stable против всех остальных

В слое `*.mk` есть ровно **один** поведенческий свитч Stable-vs-other
(`bootable/recovery/orangefox.mk:151-153`, точное совпадение с учетом
регистра `Stable`). Всё остальное, завязанное на тип сборки, — только
строки, пропсы и имя zip.

| `FOX_BUILD_TYPE=` | Конкретные эффекты |
|---|---|
| `Stable` (дефолт нашего дерева: `pixels/build.sh --build-type`, без аргумента — `Stable`; `pixels/vendorsetup.sh` сохраняет предустановленное значение или дефолтит) | `OF_ADVANCED_SECURITY:=1` → `-DOF_ADVANCED_SECURITY="1"` (`orangefox.mk:152,224-225`). Рантайм: §2. Имя zip `...-Stable-...` (`vendor/recovery/OrangeFox_A14.sh:330/332`). `ro.orangefox.type=Stable` (`twrp.cpp:459`), строки About/`fox_build_type1` (`data.cpp:736,898`), тип в `releaseinfo.json` (`twrp-functions.cpp:367`), приветствие "Build type: Stable + ссылка на поддержку" (`twrp-functions.cpp:2735-2738`). |
| `Beta` / `Testing` / `Unstable` / любое другое непустое | **Нет** `OF_ADVANCED_SECURITY` → штатный путь автостарта MTP (`twrp.cpp:292-313`), нет раннего `ctl.stop adbd`. `BETA` (в любом регистре) всё равно получает строку со ссылкой на поддержку (`twrp-functions.cpp:2736`); остальные значения печатают "No official support for unknown builds" (`:2740`). Больше ничего в рантайме не меняется. |
| не задан / пусто | Дефолт `"Unofficial"` (`orangefox.mk:143`, `OrangeFox_A14.sh:311`). Тот же рантайм, что у Beta, плюс предупреждение Unofficial (`twrp-functions.cpp:2732-2733`). |

`FOX_VARIANT` (`pixels/vendorsetup.sh:150` → `default`; `orangefox.mk:43-47`)
и `OF_MAINTAINER` (`:151` → `LeeGarChat`; `orangefox.mk:206-210`,
без значения = `"Testing build (unofficial)"`) — **только строки**
(`ro.orangefox.variant`, страница About, имя zip, `releaseinfo.json`).
Поведенческих форков нет. `FOX_BUILD` (`R12.0[_N]`, `orangefox.mk:24-37`,
`--patch N` через `pixels/build.sh:196`) — то же, только версия.

---

## 2. `OF_ADVANCED_SECURITY=1` — полный список гейтов

Выставляется автоматически на `Stable`; ручной
`export OF_ADVANCED_SECURITY=1` идёт тем же путём. Старое имя
`FOX_ADVANCED_SECURITY` — жёсткая ошибка сборки (`orangefox.mk:616-617`).

| Гейт | Эффект на практике |
|---|---|
| `twrp.cpp:246-249` | **`ctl.stop adbd` на раннем старте, `orangefox.adb.status=0`. ADB мёртв с рождения — включая экран ввода пароля.** |
| `twrp.cpp:287-290` | `fox_advanced_security=1`, **`tw_mtp_enabled=0`**, штатный автостарт MTP пропущен (`:292-314` ветка else мертва). В лог: `ADB & MTP disabled by maintainer`. |
| `main.xml:48-65` | Отложенное включение **при первом входе на страницу `main`**: `startmtp` + `adb enable` срабатывают один раз (условия: нет пароля локскрина, `adb_startup=1`, `adb_started!=1`, `tw_has_mtp=1`). **На зашифрованном девайсе, висящем на вводе пароля, `main` не достигается → adb/MTP молчат весь сеанс.** |
| `partitionmanager.cpp:812-819` (дерево pixels) | Только intent-комментарий: рестарта MTP после metadata decrypt нет (и так покрыто `tw_mtp_enabled=0`, но зафиксировано явно). |

**Явно НЕ затронуто** (проверено grep — работает одинаково на Stable):
- ADB sideload / ORS (`openrecoveryscript.cpp:408-448` делает `ctl.start adbd`; автостарт ORS `twrp.cpp:280-282` безусловен).
- Встроенный терминал / nano (`data.cpp:1513-1518`, важен только `TW_EXCLUDE_NANO`).
- Файловый менеджер (условий на `fox_advanced_security` нет).
- Страница `pass.xml:92-106` проверки пароля (смотрит только `adb_startup`/`tw_mtp_enabled`).

Практический вывод для тестеров: на Stable «нет adb на экране пароля» — это
**by design**, а не баг. Наш self-heal в `otg.rs` (`switch_to_device`:
`ctl.start adbd` + явный rebind UDC) осознанно оживляет его ради отладки.
Передёргивание кабеля помочь не может: init-триггеры edge-based, а нового
фронта, пока висит экран пароля, не приходит никогда.

---

## 3. Инвентарь флагов (по группам)

Легенда: **tree** = выставляет `pixels/` (vendorsetup.sh, если не указано иное).
"Default" = поведение без установки.

### 3.1 A/B + раскладка recovery (tree: всё выставлено)

| Флаг | Значения / читается | Эффект | Tree |
|---|---|---|---|
| `FOX_AB_DEVICE` | `1`; `orangefox.mk:93,156-176` | Режим A/B: линковка bootctl, пути OTA/update-engine | `=1` (:158; также форсится `BoardConfig.mk:35` `AB_OTA_UPDATER`) |
| `FOX_VIRTUAL_AB_DEVICE` | `1`; `orangefox.mk:87-95` | Virtual-A/B: тянет `FOX_AB_DEVICE=1` + `FOX_VANILLA_BUILD=1`; гейтит KernelSU, VAB ORS wipe-format | `=1` (:157) |
| `FOX_VENDOR_BOOT_RECOVERY` | `1` (экспериментально, с варнингом); `orangefox.mk:179-195` | Recovery в vendor_boot: форсит AB + `OF_NO_SPLASH_CHANGE` + vanilla; раскладка repacker/ramdisk под vendor_boot | `=1` (:159; соответствует `BoardConfig.mk:236-237`) |
| `FOX_RECOVERY_VENDOR_BOOT_PARTITION` | путь к блочнику; `OrangeFox_A14.sh:664-666` (только shell-постобработка) | Переписывает `VENDOR_BOOT_PARTITION=` на реальный узел `vendor_boot` | по семьям (:161-167): gs201/gs101→`14700000.ufs`, malibu→`3c2d0000.ufs`, laguna→`3c400000.ufs`, иначе `13200000.ufs` |
| `FOX_TARGET_DEVICES` / `TARGET_DEVICE_ALT` | списки через запятую; `orangefox.mk:310-317` | Assert/allow-лист (`ro.twrp.target.devices`, проверки OTA) | оба `="$_ALL_DEVS"` (:175-176, discovery, не хардкод) |

### 3.2 Vanilla / MIUI (tree: vanilla включён)

| Флаг | Эффект | Tree |
|---|---|---|
| `FOX_VANILLA_BUILD=1` (`orangefox.mk:94,103-114`) | Пропуск всех MIUI/OrangeFox-патчингов (каскад `OF_SKIP_*`/`OF_DISABLE_*`/`OF_DONT_*`) | `=1` (:171) |
| `OF_DISABLE_MIUI_SPECIFIC_FEATURES=1` (`:106,117-125`) | Вырезает MIUI-меню/патчинг | `=1` (:172) |

### 3.3 Identity / версия (только строки)

`FOX_BUILD_TYPE` (§1, через `build.sh --build-type TYPE`, дефолт `Stable`),
`FOX_VARIANT`, `OF_MAINTAINER` (§1),
`FOX_MAINTAINER_PATCH_VERSION` (только целые числа, иначе ошибка сборки;
`build.sh --patch N`; дописывает `_N` к `R12.0`).

### 3.4 Геометрия UI (tree: всё выставлено)

| Флаг | Дефолт | Tree |
|---|---|---|
| `OF_SCREEN_H` | `1920` | `=2400` (:181; рантайм-оверрайд `DOF_SCREEN_H` всё равно побеждает) |
| `OF_STATUS_H` | `72` | zumapro→`150`, остальным `130` (:183-190) |
| `OF_STATUS_INDENT_LEFT/RIGHT` | `20` | `=80/80` (:191-192) |
| `OF_HIDE_NOTCH` | `0` | `=1` (:193) |
| `OF_CLOCK_POS` | `0` | `=1` (:194) |
| `OF_ALLOW_DISABLE_NAVBAR` | `1` | `=0` (:195) |
| `OF_OPTIONS_LIST_NUM` | — | `=6` (:196) |

### 3.5 Сжатие / ramdisk

| Флаг | Эффект | Tree |
|---|---|---|
| `OF_USE_LZ4_COMPRESSION=1` | LZ4-ramdisk + кодовый путь (`BOARD_RAMDISK_USE_LZ4`) | `=1` (:199; `BoardConfig.mk:110` согласен) |
| `FOX_USE_LZ4_COMPRESSION`, `FOX_USE_LZMA_COMPRESSION`, `FOX_ADVANCED_SECURITY`, `OF_PATCH_VBMETA_FLAG`, `OF_TARGET_DEVICES` | **устарели — жёсткие ошибки сборки** (`orangefox.mk:600-621`) | корректно отсутствуют |

### 3.6 Переключатели фич (tree)

| Флаг | Эффект | Tree |
|---|---|---|
| `OF_NO_TREBLE_COMPATIBILITY_CHECK=1` | Пропуск Treble-проверки | `=1` (:203) |
| `OF_ENABLE_LPTOOLS=1` | Включает `lptools` (`TW_INCLUDE_LPTOOLS`, нужен `external/lptools`) | `=1` (:204; `BoardConfig.mk:225` дублирует) |
| `OF_USE_GREEN_LED=0` | Зелёный LED выкл (`-DOF_NO_GREEN_LED`) | `=0` (:205) |
| `OF_NO_SPLASH_CHANGE=1` | Прячет меню смены сплэша (также авто-форсится vendor-boot recovery) | `=1` (:206) |
| `OF_RECOVERY_AB_FULL_REFLASH_RAMDISK=1` | Полный reflash ramdisk на A/B (`twrpRepacker.cpp:191,357`) | `=1` (:207) |
| `OF_USE_DMCTL=1` | dmctl вместо dmsetup | `=1` (:213) |
| `FOX_USE_BASH_SHELL=1` | Кладёт bash в recovery (`OrangeFox_A14.sh:156,368,422`) | `=1` (:227) |
| `FOX_ENABLE_APP_MANAGER=1` / `FOX_DELETE_AROMAFM=1` | Менеджер приложений вкл / AROMA-FM удалён (`:746`) | `=1` (:239-240) |
| `FOX_ENABLE_KERNELSU_SUPPORT=1`, `..._NEXT_SUPPORT=1` | Поддержка KernelSU/Next (нужен vAB) | `=1` (:258-259) |
| `FOX_DELETE_INITD_ADDON=1` | Удаляет init.d-аддон (включено по умолчанию, если не `=0`) | `=1` (:262) |
| `OF_QUICK_BACKUP_LIST` | Пресет быстрого бэкапа (`tw_backup_list_quick`) | `="/boot;/vendor_boot;/data;"` (:243) |
| `OF_UNBIND_SDCARD_F2FS=1` | Bind-unmount `/sdcard` перед F2FS-ремонтом/форматом | `=1` (:244) |
| `OF_BIND_MOUNT_SDCARD_ON_FORMAT=1` | — | `=1` (:245) |
| `OF_DYNAMIC_FULL_SIZE=8531214336` | Константа full-size (совпадает с `BOARD_SUPER_PARTITION_SIZE`) | (:246) |
| `OF_USE_LEGACY_BATTERY_SERVICES=1` | Legacy battery HAL (`TW_USE_LEGACY_BATTERY_SERVICES`) | `=1` (:249) |
| `OF_FORCE_DATA_FORMAT_F2FS=1` | Формат только в F2FS | `BoardConfig.mk:215` (mk, не env) |
| `OF_FL_PATH1="cmd:/system/bin/recovery-pixel-boot torch"` | Хук фонарика на наш Rust-демон | (:216) |
| `OF_WORKAROUND_BACKUP_BUG=1` | Форсирован `1` в mk | дефолт (не трогаем) |
| `OF_DONT_KEEP_LOG_HISTORY=0`, `FOX_INSTALLER_DISABLE_AUTOREBOOT=0`, `FOX_USE_DATA_RECOVERY_FOR_SETTINGS=0` | Явное выключение (дефолты отличались бы) | (:252-254) |

### 3.7 MTP / USB / crypto (tree, сторона mk)

`TW_THEME:=portrait_hdpi` (`BoardConfig.mk:192`),
`TW_EXCLUDE_APEX/DEFAULT_USB_INIT/TWRPAPP:=true` (`:205-207`),
`TW_USE_TOOLBOX`, `TW_INCLUDE_CRYPTO/CRYPTO_FBE/FBE_METADATA_DECRYPT`,
`TW_INCLUDE_FASTBOOTD/RESETPROP/REPACKTOOLS/NTFS_3G/FUSE_* /LPTOOLS`,
`TWRP_INCLUDE_LOGCAT`, `TW_USE_FSCRYPT_POLICY:=2` (`BoardConfig.mk:196-225`).
`OF_CHECK_OVERWRITE_ATTEMPTS` не тронут → проверка включена.
`OF_SKIP_FBE_DECRYPTION` закомментирован / `OF_DEFAULT_KEYMASTER_VERSION`
явно unset (`vendorsetup.sh:220`).

### 3.8 Тулчейн / прочее (tree)

`USE_CCACHE=1` (:152), `TARGET_ARCH=arm64` (:153, `BoardConfig.mk:65`
всё равно перебивает env), `FOX_REPLACE_TOOLBOX_GETPROP=1` (:226),
`FOX_BASH_TO_SYSTEM_BIN=1` (:228), `FOX_LOCAL_CALLBACK_SCRIPT` (:265),
`FOX_KERNEL_VER` / `FOX_KEYMINT_TYPE` / `DEVICE_BUILD_FLAG` / `LGZ_LEVEL`
(через `build.sh`), `VENDOR_BOOT_PATCH_STOCK` (путь gs101).
Закомментировано (сознательно выкл): `FOX_ASH_IS_BASH`,
`FOX_USE_TAR_BINARY`, `FOX_USE_SED_BINARY`, `FOX_USE_XZ_UTILS`,
`FOX_USE_UPDATED_MAGISKBOOT`, `FOX_USE_LZ4_BINARY` (`:229-235`).

### 3.9 Выставлено, но нигде не читается (мёртвые ручки в этом дереве)

`OF_IGNORE_LOGICAL_MOUNT_ERRORS=1` (`:202`) — читателя нет ни в
`bootable/`, ни в `vendor/recovery/`, ни в `vendor/twrp/`. Legacy/no-op здесь.
(Заданное-но-неиспользуемое в других местах: `FOX_REPLACE_BOOTIMAGE_DATE`,
`OF_KEEP_DM_VERITY*` (в mk и так форсирован `1`), `OF_FORCE_DISABLE_DM_VERITY`,
`OF_FIX_OTA_UPDATE_DENSITY_ERROR`, `OF_NO_MIUI_OTA_WARNING`,
`FOX_BUGGED_AOSP_ARB_WORKAROUND`, `FOX_RECOVERY_SYSTEM/VENDOR_PARTITION`,
`FOX_SETTINGS_ROOT_DIRECTORY*`, `OF_SKIP_DECRYPTED_ADOPTED_STORAGE`.)

---

## 4. Окружение vs mk-файлы

- **Через окружение** (экспорт до/во время lunch; читают и make, и
  `OrangeFox_A14.sh` на пост-обработке): всё, что экспортирует
  `vendorsetup.sh` (`FOX_BUILD_TYPE/VARIANT`, `OF_MAINTAINER`,
  AB/VAB/vendor-boot/vanilla-семейства, `FOX_TARGET_DEVICES`,
  `TARGET_DEVICE_ALT`, все `OF_SCREEN/STATUS/*`, `OF_USE_LZ4`,
  `OF_ENABLE_LPTOOLS`, `OF_USE_*`, `FOX_USE_BASH_SHELL`,
  `FOX_ENABLE_APP_MANAGER`, `FOX_DELETE_*`, `OF_QUICK_BACKUP_LIST`,
  `OF_UNBIND_SDCARD_F2FS`, `FOX_ENABLE_KERNELSU*`,
  `FOX_RECOVERY_VENDOR_BOOT_PARTITION`, `USE_CCACHE`, `TARGET_ARCH`,
  `DEVICE_BUILD_FLAG`) плюс экспорты `build.sh`
  (`FOX_MAINTAINER_PATCH_VERSION`, `FOX_KERNEL_VER`, `FOX_KEYMINT_TYPE`,
  `LGZ_LEVEL`, `VENDOR_BOOT_PATCH_STOCK`).
- **Обязаны лежать в mk-файлах** (Soong/mkbootimg/наследование продукта
  читают их там; env перебивается): `TARGET_ARCH` (`BoardConfig.mk:65` `:=`),
  все `BOARD_*` (версия хедера, LZ4, размеры разделов, metadata, vendor-boot
  moves), `TARGET_RECOVERY_*`, `AB_OTA_*`, `PRODUCT_PLATFORM`, все `TW_*`,
  `OF_FORCE_DATA_FORMAT_F2FS`.

---

## 5. Практическая шпаргалка

- Нужна нормальная (не Secure) отладочная сборка: `build.sh --build-type Beta`
  (или что угодно кроме `Stable`) → adbd жив на старте, MTP автостарт,
  остального рантайма ноль. В zip будет `-Beta-`, в приветствии ссылка
  на поддержку.
- Stable + adb на экране пароля: только вход в `main`, ручной тумблер ADB
  в Advanced, ORS/sideload (`ctl.start adbd`) или наш otg self-heal.
  Реплаг не помогает (edge-based init).
- `Beta` **ничего** не включает в рантайме по сравнению со Stable, кроме
  *снятия* гейтов §2. Никаких "бета-фич" флагом нет.
- Устаревшие имена из §3.5 не ставить никогда — мгновенная ошибка сборки
  by design.
