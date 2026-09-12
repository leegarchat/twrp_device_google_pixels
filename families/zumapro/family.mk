# families/zumapro/family.mk — Tensor G4.
# Included by BoardConfig.mk when DEVICE_BUILD_FLAG=zumapro.

VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \
        earlycon=exynos4210,0x10870000 \
        console=ttySAC0,115200 \
        androidboot.console=ttySAC0 printk.devkmsg=on \
        cma_sysfs.experimental=Y \
        cgroup.memory=nokmem \
        rcupdate.rcu_expedited=1 \
        rcu_nocbs=all \
        rcutree.enable_rcu_lazy \
        swiotlb=noforce \
        disable_dma32=on \
        sysctl.kernel.sched_pelt_multiplier=4 \
        kasan=off \
        at24.write_timeout=100 \
        log_buf_len=1024K bootconfig"
BOARD_BOOTCONFIG += androidboot.usbcontroller=11210000.dwc3
BOARD_BOOTCONFIG += androidboot.boot_devices=13200000.ufs
BOARD_BOOTCONFIG += androidboot.load_modules_parallel=true

# Partitions - Blocks
BOARD_FLASH_BLOCK_SIZE := 4096
