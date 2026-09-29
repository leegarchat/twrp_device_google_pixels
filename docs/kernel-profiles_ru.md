> [English version](kernel-profiles.en.md)

# Kernel-профили (удалены)

Kernel-профилей в дереве больше нет: стоковое ядро сохраняется всегда,
собирается только универсальный рамдиск-пейлоад, и компоновать
`VENDOR_CMDLINE` не для чего. В `BoardConfig.mk` — только dummy-cmdline
для промежуточного `vendor_boot` (он отдаёт platform-фрагмент, из
которого извлекается пейлоад).

Что удалено при переходе на только-AIO:

- секции `kernels` / `default_kernel` в `families/*/family.json` и
  per-device оверрайды `_kernels_disabled` в `devices/*/pixel.json`;
- `include/prebuilt/gen_kernel_mk.py` (`--list` / `--fingerprint` /
  `--generate`) и генерированные `families/*/.gen_kernel.mk`;
- `build.sh -k/--kernel` / `--force`, цикл kernel-групп и погрупповые
  `*.img`-артефакты;
- запись `kernel_bootcfg` в `/pixelrunatboot.json` (reflash пересобирает
  каждый слот из его же стокового образа, сохраняя header/cmdline/dtb).

Файл оставлен, чтобы не бить ссылки карты документации.
