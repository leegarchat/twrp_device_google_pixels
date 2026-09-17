> [English version](decrypt.en.md)
# Расшифровка data: keymint, weaver, vold

Всё собирается из исходников — пребилд-HAL'ов в дереве нет. Сток нужен
только как референс (fstab, cmdline, карта разделов).

## KeyMint HAL (per-family, из исходников)

| Семья | Тип | Почему |
|---|---|---|
| `gs201`, `gs101` | `cpp` | C++ HAL против старого Trusty TA |
| `zuma`, `zumapro`, `laguna`, `malibu` | `rust` | Rust HAL против нового TA |

- Тип задаётся полем `keymint` в `family.json` → `FOX_KEYMINT_TYPE`
  (читают `build.sh`, `device.mk`, колбэк через `.build_platform.conf`).
- Колбэк инжектит нужный сервис в rc и гасит чужой; keymint-модуль
  собирается отдельным `mka` **первым** (порядок важен).
- C++ на zuma проверен и отвергнут (restart loop, TA mismatch);
  Rust-from-source зелёный (md5 out==device, `User 0 Decrypted`).
- VINTF-манифесты keymint — только schema 2.0 (recovery везёт libvintf
  8.0; стоковая schema 9.0 роняет регистрацию **всех** device-HAL).
  Версии: zuma/zumapro `IKeyMintDevice v5+RPC v3`, gs201 `v4+v3`,
  malibu/laguna `v5+v3`. Поле `recovery_available` для Rust HAL
  сохраняется (иначе сервис не стартует в рекавери).
- Health HAL не тянем вовсе (батарея в рекавери не обслуживается).

## Weaver и Titan M3 (`recovery-tensor-daemon`)

Rust-демон (`include/recovery-tensor-daemon/`: `common/`, `storageproxy/`,
`weaver/`) обслуживает FBE synthetic-password через weaver-слоты.

- Titan M (старые SoC): protobuf-путь, не тронут.
- Titan M3 (новые): raw-протокол (`weaver/m3.rs`, hdr `0x000e0000`) как
  фолбэк с лэтчем в `service.rs`: первый успех фиксирует путь.
- Со стороны vold: парсинг packed 5-байтного `.weaver`-файла (слот —
  offset 1, big-endian) и фолбэк `keySize 0→16` (Titan M3 в рекавери
  отдаёт 0; стандарт — 16). Проверка юнитами в демоне.

## Vold multi-device (`MetadataCrypt` + libfstab)

Стоковый vold умеет один userdata-девайс; на новых Pixel'ах — zoned +
alias-набор. Наши патчи:

- libfstab: парсинг `device=zoned:` / `device=exp:` / `exp_alias:`,
  поле `user_devices` в fstab.
- `MetadataCrypt`: multi-device маппинг, dm-имена и key-подкаталоги по
  basename (`zoned_device`), таймаут метадаты 30→120.
- `partitionmanager.cpp`: `FscryptMountMetadataEncryptedWithTimeout(…, 120)`
  + гейт по состоянию keymint-сервиса (нет сервиса — скип вместо виса).
- fstab'ы: пре-рендеренные под vendor_ramdisk в `families/<fam>/fstab/`,
  рекавери-fstab — `recovery.fstab`.

## KDF-плейграунд (`fbe_kdf/`)

`fox_fbe_kdf <secret-hex> "<pipeline>"` — исследование FBE
synthetic-password KDF: стадии `slice`/`hex`/`unhex`/`ph512`/`ph256`/
`xorhalf`/`sp800` слева направо, первая успешная строка
`system/etc/fox_kdf.conf` побеждает. Переопределение на девайсе без
пересборки: `/tmp/fox_kdf.conf` через adb push. Это инструмент
исследования, не штатный путь расшифровки.
