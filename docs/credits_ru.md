> [English version](credits.en.md)
# Заимствования и референсы

## yogi-orangefox (Pixel 11 / malibu)

Часть логики P11-серии подсмотрена и адаптирована из
https://github.com/asdfmonster261/yogi-orangefox (unified-дерево под
malibu: yogi = 11 Pro Fold, cubs/grizzly/kodiak = 11/11 Pro/11 Pro XL).

Что взято:

- vold multi-device metadata decrypt: парсинг `device=zoned:` /
  `device=exp:`/`exp_alias:` в libfstab, поле `user_devices`, dm-имена
  и key-подкаталоги по basename, 5-байтный `.weaver`-слот (BE@1),
  фолбэк keySize 0→16, таймаут метадаты 30→120;
- VINTF keymint-манифест schema 2.0 (recovery везёт libvintf 8.0,
  schema 9.0 из стока роняет регистрацию ВСЕХ device-HAL);
- протокол Titan M3 weaver (raw-структуры, hdr `0x000e0000`) —
  перенесён в Rust-демон как фолбэк с лэтчем, protobuf-путь Titan M
  не тронут;
- cover-панель фолдов (`graphics_drm.cpp`: выбор DSI с большим
  `connector_type_id`);
- референс тач-стека/карты девайсов.

Что НЕ взято (сознательно): v5-HAL пребилдом из стока (собираем
Rust-HAL из исходников), OTG через vendor aocd (наш native-путь),
попереслотный reflash (наш путь — strip dtb + json-cmdline), хардкод
яркости cover-панели (подсветка находится сама), health-HAL пребилдом
(не тянем вовсе), `otg_*.sh`-скрипты (устарели против `init.rs`).

## comet_test_fold (Pixel 9 Pro Fold)

Отсюда — hinge-детект (EV_SW), `DOF_*`-геометрия, letterbox-движок
(`data.cpp`/`pages.cpp`), vold multi-device portions, M3-фолбэк,
touch negotiator/offload. P11-покрытия hinge/letterbox не имел —
здесь мы впереди обоих источников.

## Стоковые прошивки

Только референс: cmdline `vendor_boot` (побайтово в `kernels`),
`vendor_dlkm` (порядок `touch_modules`), fstab (пре-рендер).
Би snapshots стока в образ не едут. Разбор — вне дерева
(`roms_extract_pixel/` + `stock_refresh.py` у мейнтейнера).
