# families/gs201/family.mk — Tensor G2.
# Included by BoardConfig.mk when DEVICE_BUILD_FLAG=gs201.

VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \
        earlycon=exynos4210,0x10A00000 \
        console=ttySAC0,115200 \
        androidboot.console=ttySAC0 \
        printk.devkmsg=on \
        swiotlb=noforce \
        cma_sysfs.experimental=Y \
        cgroup_disable=memory \
        rcupdate.rcu_expedited=1 \
        androidboot.usbcontroller=11210000.dwc3 \
        rcu_nocbs=all \
        stack_depot_disable=off \
        page_pinner=on \
        swiotlb=1024 \
        disable_dma32=on \
        at24.write_timeout=100 \
        log_buf_len=1024K \
        bootconfig"
BOARD_BOOTCONFIG += androidboot.usbcontroller=11210000.dwc3
BOARD_BOOTCONFIG += androidboot.boot_devices=14700000.ufs
BOARD_BOOTCONFIG += androidboot.load_modules_parallel=true

# Partitions - Blocks
BOARD_FLASH_BLOCK_SIZE := 131072
