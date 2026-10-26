> [English version](credits.md)

# Заимствования и референсы

`Гайд v3 · 2026-10-26 · что взято, что пересобрано`

## yogi-orangefox (Pixel 11 / malibu)

Часть логики P11-серии подсмотрена и адаптирована из https://github.com/asdfmonster261/yogi-orangefox (unified-дерево под malibu: yogi = 11 Pro Fold, cubs/grizzly/kodiak = 11/11 Pro/11 Pro XL).

**Что взято:**

- vold multi-device metadata decrypt: парсинг `device=zoned:` / `device=exp:`/`exp_alias:` в libfstab, поле `user_devices`, dm-имена и key-подкаталоги по basename, 5-байтный `.weaver`-слот (BE@1), фолбэк keySize 0→16, таймаут метадаты 30→120;
- VINTF keymint-манифест schema 2.0 (recovery везёт libvintf 8.0, schema 9.0 из стока роняет регистрацию **ВСЕХ** device-HAL);
- протокол Titan M3 weaver (raw-структуры, hdr `0x000e0000`) - перенесён в Rust-демон как фолбэк с лэтчем, protobuf-путь Titan M2 не тронут;
- fold cover-панель (`graphics_drm.cpp`: выбор DSI с большим `connector_type_id`);
- референс touch-стека / device-map.

**Что НЕ взято (осознанно):**

- v5 HAL стоковым пребилдом (собираем Rust HAL из исходников), OTG через вендорский aocd (наш нативный путь), кросс-слот reflash (наш путь - strip dtb + json cmdline), захардкоженная яркость cover-панели (подсветка находит себя сама), health HAL пребилдом (собираем generic sysfs `health@2.1-service` из исходников), скрипты `otg_*.sh` (устарели против `init.rs`).

<details>
<summary>📖 comet_test_fold (Pixel 9 Pro Fold) + стоковая прошивка</summary>

- Отсюда - hinge-детект (EV_SW), геометрия `DOF_*`, letterbox-движок (`data.cpp`/`pages.cpp`), куски мультидевайс vold, M3-фолбэк, touch-негоциатор/offload. В P11-покрытии не было hinge/letterbox - здесь мы впереди обоих источников.
- **Стоковая прошивка** - только референс: cmdline `vendor_boot` (побайтово в `kernels`), `vendor_dlkm` (порядок `touch_modules`), fstab (пре-рендер). Стоковые бинарные снапшоты в образ не едут. Анализ - вне дерева (`roms_extract_pixel/` + `stock_refresh.py` у мейнтейнера).

</details>
