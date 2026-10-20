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
| `install-rom <args>` | Legacy entry point (kept as alias) | Forwards to `/system/bin/fox-hsinstall` (cargo-built Rust, see below); the installer calls the engine directly now. |

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

`siw`/`iw`/`lptools_new` are static arm64 utilities: streaming
partition reads without mounting (`siw read` → `iw read`) plus DM
mapping (`siw map` with settle-wait and siw-UUID ownership, `lptools_new
--map` via the system fs_mgr stack). Laguna AoC staging is mapper-only:
a block counts as existing only as a live `/dev/block/mapper/` node
cross-checked to a dm device (`/dev/block/by-name/*` is never used —
static, may dangle); order is mount-present → `siw map` → `lptools_new
map` → `iw` direct read (once per boot, last resort). Only devices
mapped by the current call are ever unmapped — foreign (first-stage /
TWRP / lptools) mappings are used read-only. The `otg-patch` oneshot
only mounts existing nodes: it creates nothing, removes nothing.
Stock `runatboot.sh` is an empty OFox hook (invoked by
`twrp.cpp`); an extension point for addons.

## High-speed install (`fox-hsinstall` + installer branch)

Opt-in (`tw_high_speed_flash` checkbox on the flash-confirm page, A/B
devices only, default off). `twinstall` detects a payload-based OTA and
runs `recovery-pixel-boot install-rom <zip>` over the same updater pipe
protocol (`progress`/`set_progress`/`ui_print`); stock `update_engine`
stays the fallback.

Engine (`rust_projects/hs-install`, static musl arm64 prebuilt):
probe zip → `pre-device` vs `ro.product.device` (hard mismatch error,
never falls back) → full-OTA verdict (incremental → exit 2, stock path)
→ helpers `siw`+`lptools_new` present (else exit 5, stock path) →
`siw info -f json` live state → phase-2 fit ladder
(`FITS_AS_IS → KILL_ONE → KILL_TWO → WIPE_ALL → IMPOSSIBLE`, kills from
the current slot largest-first) → backup critical weights 2+4 to `/tmp`
(super/modem excluded by design) → single-threaded group rebuild via
`siw` (unmap, remove + create in both metadata slots, map writable) →
batches 1→2→3→4 with LPT order, per-thread sha256 validation and a
barrier per batch → on failure finish the batch, roll back, exit 1
(slot never switched by the engine) → `fsync` + `sync`.

On engine exit 0 the installer switches the slot (`bootctl
setActiveBootSlot`) and completes normally. The engine never formats
`/data`: the downgrade-format trigger is deliberately not copied.

## High-speed engine `fox-hsinstall` (Rust, static musl)

Cargo-built `hs-install` (`include/source/`, outside Soong), shipped as
a static prebuilt in `recovery/root` (same mechanism as `siw`).
LP surgery goes through forked `/system/bin/siw` — the kernel guard
silently drops raw super writes from any other binary, so direct
`liblp` writes are impossible by design. Manifest parsing is pure Rust.
Codecs: `lzma-rust2` + `ruzstd` + `crabz2` (pure Rust, no C).
Update flow: probe → device gate (`pre-device`) → full-OTA verdict
(+ `minor_version` delta signal, exact-tiling check) → live super
(`siw info` JSON) → fit ladder → backup critical weights 2+4 →
unmap of target + stale nodes (live slot untouched) → group rebuild
→ op-level thread pool (auto `min(ncpu,8)` + RAM cap) with per-op
input hashes → barrier per batch → `fsync`, re-read sha256 vs
`new_partition_info.hash` (optional `--readback` re-verifies from
storage) → rollback ledger on weights 2/3/4 failure only → `sync`,
`/tmp` cleanup.
Reports progress over stdout (updater pipe protocol: staged user
lines, `set_progress`, `log` diagnostics) for the twinstall loop,
which relays it live to the GUI console (reflash pattern).
