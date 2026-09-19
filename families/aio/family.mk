# families/aio/family.mk — all-in-one (universal) build.
#
# No kernel/cmdline/bootconfig of its own: the stock kernel is kept and only
# the recovery ramdisk cpio is delivered (build.sh -c). The intermediate
# vendor_boot image only donates its recovery fragment, so the cmdline guard
# in BoardConfig.mk is bypassed for aio (dummy VENDOR_CMDLINE).
# Layout follows the default split (recovery fragment), like all families
# except gs101; the installer places the cpio into the right stock fragment
# per target device.

# Partitions - Blocks (image sizing only, same as most families)
BOARD_FLASH_BLOCK_SIZE := 131072
