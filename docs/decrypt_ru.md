> [English version](decrypt.md)

# Расшифровка data: KeyMint, weaver, vold

`Гайд v3 · 2026-10-26 · крипта из исходников`

Всё собирается из исходников - **пребилд-HAL'ов** в дереве нет. Сток нужен только как референс (fstab, cmdline, карта разделов).

## KeyMint HAL - кто вас расшифровывает

| Семейство | Тип | Почему |
|---|---|---|
| `gs101`, `gs201` | `cpp` | C++ HAL против старого Trusty TA |
| `zuma`, `zumapro`, `laguna`, `malibu` | `rust` | Rust HAL против нового TA |

- Тип задаётся полем `keymint` в `family.json`. Сборка везёт **оба** HAL в универсальном пейлоаде; при установке свап (`include/aio/aio_swap.sh`, в desktop-потоке тоже) оставляет HAL и VINTF-манифест под твою семью, остальные удаляет, а `ro.recovery.keymint=<rust|cpp>` прописывает. HAL, не совпавший с Trusty TA, тихо выходит в рантайме.
- Оба keymint-модуля собираются отдельным `mka` **первыми** (порядок важен - колбэк рекавери стейджит свежесобранные бинари из `$PRODUCT_OUT`).
- C++ на zuma проверен и **отвергнут** (restart loop, TA mismatch). Rust-from-source зелёный (`md5 out==device`, `User 0 Decrypted`).

<details>
<summary>🔌 VINTF-манифесты - схема, роняющая всё</summary>

- Рекавери везёт libvintf 8.0; стоковая schema 9.0 **роняет регистрацию ВСЕХ device-HAL** - поэтому KeyMint-манифесты остаются schema 2.0.
- Версии: zuma/zumapro `IKeyMintDevice v5+RPC v3`, gs201 `v4+v3`, malibu/laguna `v5+v3`. Флаг `recovery_available` живёт в Soong `.bp`-файлах (KeyMint/keymaster/weaver `Android.bp`), а не в VINTF-манифесте - именно он даёт сервису стартовать в рекавери.

</details>

<details>
<summary>🔐 Weaver + Титан (`recovery-tensor-daemon`)</summary>

- Rust-демон (`include/recovery-tensor-daemon/`: `common/`, `storageproxy/`, `weaver/`) отдаёт FBE synthetic-пароли через weaver-слоты. Мультиколл: `storageproxy` (Trusty TIPC + UFS RPMB по SG_IO; RPMB-узел автодетектится, кеш в `/tmp/.rpmb_sg_dev`) + `weaver` (Titan M2/M3 → `android.hardware.weaver.IWeaver/default`) + `run` (оба в параллельных тредах) + `titan-wipe`.
- **Titan M2** (полевое соответствие, Pixel 6-9: gs101/gs201/zuma/zumapro): protobuf-путь, не тронут. **Titan M3** (полевое соответствие, Pixel 10/11: laguna/malibu): raw-протокол (`weaver/m3.rs`, hdr `0x000e0000`) как фолбэк с лэтчем - первый успех пинит путь (`service.rs`). Геометрия апплета (слоты, размеры key/value) **не** захардкожена: учится из ответа апплета `getConfig`, кешируется, каждый read/write строго валидируется по ней.
- Сторона vold: парсинг упакованного 5-байтного `.weaver` (слот на оффсете 1, big-endian) + фолбэк `keySize 0→16` (Titan M3 в рекавери возвращает 0; стандарт - 16). Покрыто юнит-тестами демона.
- Вайпа Титана при обычной расшифровке нет - `titan-wipe` только внутри маршрута Format Data (b): зеркалит стоковый factory-reset `WipeTitanM` - Nugget `NUKE_FROM_ORBIT` (`APP_ID_NUGGET 0x00`, param `0x0005`, magic `0xc05fefee`), до 5 попыток. Ветвления по поколениям нет (M1/M2/M3 делят Nugget-протокол; транспорт сам выбирает `/dev/citadel0` vs `/dev/gsc0`). Вызывающий стопает `recovery_weaver` до `--erase` и рестартует после. См. [диагностику](diagnostics_ru.md).

</details>

<details>
<summary>💾 Мультидевайс vold (`MetadataCrypt` + libfstab)</summary>

- Стоковый vold умеет один userdata-девайс; на новых Pixel это zoned + alias-набор. Наши патчи добавляют зеркальный парсер в `partitionmanager.cpp` (стоковый `libfstab` не тронут - тройки libfstab в дереве нет): `device=zoned:` / `device=exp:` / `exp_alias:`, поле `user_devices`, dm-имена и key-подкаталоги по basename (`zoned_device`), таймаут метадаты 30→120.
- `partitionmanager.cpp`: `FscryptMountMetadataEncryptedWithTimeout(…, 120)` + гейт по состоянию сервиса KeyMint (нет сервиса - скип вместо висения).
- Fstab'ы: компоненты first-stage `vendor_ramdisk` **не собираются** (`FOX_NO_FIRST_STAGE=1`, стоковый first_stage сохраняет установщик); посемейные `recovery.fstab` едут swap-kit'ом `recovery.fstab.<fam>` (живой файл - aio-заглушка).

</details>

<details>
<summary>🔋 Батарея в рекавери - HIDL health HAL</summary>

- Батарею обслуживает `android.hardware.health@2.1-service` (generic sysfs, из исходников, без вендорских блобов). Компилируется через `PRODUCT_PACKAGES`, в рамдиск стейджится `fox_build_callback.sh` из `$PRODUCT_OUT` - **не** через `TARGET_RECOVERY_DEVICE_MODULES` (тот список его тихо скипает: нет recovery-варианта в апстриме). Порядок зафиксирован в `BoardConfig.mk`.
- Legacy-sysfs ветка выключена осознанно (`OF_USE_LEGACY_BATTERY_SERVICES=0`). `flog.sh` теперь кладёт в каждый полевой лог снапшот батареи + health.

</details>

<details>
<summary>🧪 KDF-песочница (`include/fbe_kdf/`)</summary>

- `fox_fbe_kdf <secret-hex> "<pipeline>"` - исследование KDF синтетических паролей FBE: стадии `slice` / `hex` / `unhex` / `ph512` / `ph256` / `xorhalf` / `sp800` слева направо, побеждает первая успешная строка `system/etc/fox_kdf.conf`.
- Оверрайд на девайсе без пересборки: `/tmp/fox_kdf.conf` через adb push. Инструмент исследования - **не** стандартный путь расшифровки.

</details>
