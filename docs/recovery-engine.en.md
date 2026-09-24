> [Русская версия](recovery-engine_ru.md)

# Rust `recovery-pixel-boot` engine

Static multicall binary (`include/recovery-pixel-boot/`, `std` +
`liblibc`). Iron rule: **Rust only performs syscalls, never forks
external binaries** — the only fork point is invoking
`pixelrunatboot.sh` stages (where `resetprop`, `bootctl`, `unzip` are needed).

## Subcommands

| Subcommand | Trigger | What it does |
|---|---|---|
| `init` | `exec` on `early-init` | Device resolution → props via stage → re-resolution → swap of `<device>.twrp.flags` → `twrp.flags` fix, magiskboot unpack via stage, wait for `servicemanager.ready`, hinge detection for foldables (`DOF_*`) |
| `boot` | `exec` on `on boot` (init waits for completion — modules before GUI) | preload (`part_sysdlkm`/`preload_modules`, best-effort) → susfs fix, firmware/modules via stages + `finit_module`, haptics PM, magisk links + forked daemon, `meta-fix` via stage |
| `otg-patch` | `otg_enable` service | Inject `otg_host_shim` (`.ko` scoring), `patch_dwc3=1`, max77759 switch over I2C; Laguna uses TCPM `preferred_role`/`port_type`, then verifies host role and sourced VBUS |
| `otg-auto` | `patch_dwc3=1` trigger | VBUS host/device daemon, always resident; Laguna restores dual-role and sink preference |
| `setup-temp` | `exec` on `on init` + refresh from `boot` | `/dev/thermal_cpu` symlink: exact → `soc_therm` → auto → zone0 |
| `torch on\|off` | `OF_FL_PATH1=cmd:/system/bin/recovery-pixel-boot torch` from GUI | LM3644: I2C discovery + GPIO (v1 uAPI) + devicetree, native ioctls |

## Modules (`src/`)

| File | Area |
|---|---|
| `main.rs` | Multicall argv dispatcher |
| `config.rs` | `/pixelrunatboot.json` parser, path-key defaults |
| `init.rs` | `init` stage: resolution, props, flags, hinge/cover |
| `boot.rs` | `boot` stage: preload, device modules (including Laguna `i2c-dev`), firmware, haptics, magisk |
| `otg.rs` | OTG patch and VBUS arbitration (gate: without a PC — force host; debounced polling + settle; Laguna Type-C role controls; UDC rebind in device mode) |
| `ko_picker.rs` | `.ko` selection: kernel branch × pagesize × Android generation from `uname` (`<mod>_<ver>[_16k].ko` in the `kver/android/pagesize` hierarchy). Loading strictly via `finit_module(flags=0)` (forcing is impossible: `CONFIG_MODULE_FORCE_LOAD=n`), `EEXIST` = success |
| `props.rs`, `stage.rs` | Props and shell-stage invocation protocol |
| `i2c.rs`, `torch.rs`, `temp.rs` | I2C primitives, flashlight, thermal |

## Shell stages (`pixelrunatboot.sh` + `siw`/`iw`)

Stage protocol: argv + stdout (result) + `/tmp/recovery.log`
(diagnostics), exit 0 = success.

| Stage | Tools | What it does |
|---|---|---|
| `props-apply <family> k=v...` | `resetprop`, `setenforce` | `ro.*` props (the bionic API blocks them), gs201 flags |
| `slot-detect` | `bootctl` | Slot suffix to stdout |
| `ko-fetch <part> <sfx> <slot>` | `siw read` → `iw read` | All `*.ko` into `/dev/ko_stage/`; fallback `siw map`+mount. Depth guard: only `lib/modules/*.ko` — pagesize subdirectories (`16k-mode/`) are skipped (duplicates with foreign CRCs shadowed the flat modules and broke touch/haptics on 6.12) |
| `fw-fetch <part> <sfx> <slot>` | `siw`/`iw`, fallback mount | Firmware → `/vendor/firmware/` |
| `magiskboot-unpack <zip>` | `unzip`, `busybox` | boot/busybox into `/system/bin` |
| `meta-fix` | `mount` | Cleanup of `/metadata/ota` (waiting for the block node) |

`siw`/`iw` are static arm64 utilities (1.1M/1.4M): streaming
partition reads without mounting (`siw read`) plus DM mapping
(`siw map`, replacing `lptools_new --map`: `/dev/block/mapper/<name>`
via DM ioctl). Stock `runatboot.sh` is an empty OFox hook (invoked by
`twrp.cpp`); an extension point for addons.
