# families/gs101/family.mk — Tensor G1 (WIP, reuses gs201 fstab).
# Included by BoardConfig.mk when DEVICE_BUILD_FLAG=gs101.
# NOTE: keeps the historical double VENDOR_CMDLINE assignment verbatim
# (the second assignment wins).

VENDOR_CMDLINE := "dyndbg=\"func alloc_contig_dump_pages +p\" \\
        earlycon=exynos4210,0x10A00000 \\
        console=ttySAC0,115200 \\
        androidboot.console=ttySAC0 \\
        printk.devkmsg=on \\
        swiotlb=noforce \\
        cma_sysfs.experimental=Y \\
        cgroup_disable=memory \\
        rcupdate.rcu_expedited=1 \\
        androidboot.usbcontroller=11110000.dwc3 \\
        rcu_nocbs=all \\
        stack_depot_disable=off \\
        page_pinner=on \\
        swiotlb=1024 \\
        disable_dma32=on \\
        at24.write_timeout=100 \\
        log_buf_len=1024K \\
        bootconfig"
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
BOARD_BOOTCONFIG := androidboot.usbcontroller=11110000.dwc3
BOARD_BOOTCONFIG += androidboot.boot_devices=14700000.ufs
BOARD_BOOTCONFIG += androidboot.load_modules_parallel=true

# Partitions - Blocks
BOARD_FLASH_BLOCK_SIZE := 131072

# gs101: vendor_boot contains DLKM+DTB — must patch stock, not overwrite
VENDOR_BOOT_PATCH_STOCK := true
-include $(DEVICE_PATH)/custom_bootimg.mk
