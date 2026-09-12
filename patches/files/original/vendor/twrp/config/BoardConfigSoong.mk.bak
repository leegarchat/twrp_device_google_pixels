PATH_OVERRIDE_SOONG := $(shell echo $(TOOLS_PATH_OVERRIDE))

# Add variables that we wish to make available to soong here.
EXPORT_TO_SOONG := \
    KERNEL_ARCH \
    KERNEL_BUILD_OUT_PREFIX \
    KERNEL_CROSS_COMPILE \
    KERNEL_MAKE_CMD \
    KERNEL_MAKE_FLAGS \
    PATH_OVERRIDE_SOONG \
    TARGET_KERNEL_CONFIG \
    TARGET_KERNEL_SOURCE \
    TARGET_KERNEL_HEADERS_SOURCE \
    TW_THEME \
    TW_CUSTOM_THEME \
    DEVICE_RESOLUTION \
    TARGET_SCREEN_WIDTH \
    TARGET_SCREEN_HEIGHT \
    TARGET_RECOVERY_PIXEL_FORMAT \
    TW_EXTRA_LANGUAGES \
    TWRP_CUSTOM_KEYBOARD \
    AB_OTA_UPDATER \
    TW_TARGET_USES_QCOM_BSP \
    TARGET_RECOVERY_FORCE_PIXEL_FORMAT \
    TW_INCLUDE_JPEG \
    TW_ROTATION \
    BOARD_HAS_FLIPPED_SCREEN \
    TW_HAPTICS_TSPDRV \
    TW_SUPPORT_INPUT_AIDL_HAPTICS \
    TW_SUPPORT_INPUT_1_2_HAPTICS \
    TARGET_PREBUILT_KERNEL \
    TW_STATUS_ICONS_ALIGN \
    TW_CUSTOM_BATTERY_POS \
    TW_CUSTOM_CLOCK_POS \
    TW_CUSTOM_CPU_POS

# Setup SOONG_CONFIG_* vars to export the vars listed above.
# Documentation here:
# https://github.com/LineageOS/android_build_soong/commit/8328367c44085b948c003116c0ed74a047237a69

SOONG_CONFIG_NAMESPACES += twrpVarsPlugin

SOONG_CONFIG_twrpVarsPlugin :=

define addVar
  SOONG_CONFIG_twrpVarsPlugin += $(1)
  SOONG_CONFIG_twrpVarsPlugin_$(1) := $($1)
endef

$(foreach v,$(EXPORT_TO_SOONG),$(eval $(call addVar,$(v))))

SOONG_CONFIG_NAMESPACES += twrpGlobalVars
SOONG_CONFIG_twrpGlobalVars += \
    legacy_hw_disk_encryption \
    target_enforce_ab_ota_partition_list \
    target_init_vendor_lib \
    supports_hw_fde \
    supports_hw_fde_perf \
    tw_delay_touch_init_ms \
    tw_event_logging \
    tw_use_key_code_touch_sync \
    tw_ozip_decrypt_key \
    tw_no_screen_blank \
    tw_no_screen_timeout \
    tw_oem_build \
    tw_x_offset \
    tw_y_offset \
    tw_w_offset \
    tw_h_offset \
    tw_framerate \
    tw_round_screen \
    twrp_custom_keyboard \
    tw_support_input_1_2_haptics \
    tw_target_uses_qcom_bsp \
    tw_include_jpeg \
    recovery_touchscreen_swap_xy \
    recovery_touchscreen_flip_x \
    recovery_touchscreen_flip_y \
    recovery_graphics_force_use_linelength \
    recovery_graphics_force_single_buffer \
    twrp_event_logging \
    target_recovery_overscan_percent \
    tw_screen_blank_on_boot \
    tw_fbiopan \
    tw_ignore_major_axis_0 \
    tw_ignore_mt_position_0 \
    tw_input_blacklist \
    tw_whitelist_input \
    tw_haptics_tspdrv \
    tw_exclude_nano \
    tw_support_input_aidl_haptics \
    tw_support_input_aidl_haptics_fqname \
    tw_support_input_aidl_haptics_fix_off \
    tw_use_samsung_haptics \
    tw_brightness_path \
    tw_max_brightness

ifeq ($(TARGET_HW_DISK_ENCRYPTION),true)
SOONG_CONFIG_twrpGlobalVars += \
    hw_fde_cryptfs_hw_header_lib_name \
    hw_fde_cryptfs_hw_shared_lib_name
endif

# Defaults
TW_OZIP_DECRYPT_KEY ?= 0
TARGET_RECOVERY_OVERSCAN_PERCENT ?= 0
TW_X_OFFSET ?= 0
TW_Y_OFFSET ?= 0
TW_W_OFFSET ?= 0
TW_H_OFFSET ?= 0
TW_DELAY_TOUCH_INIT_MS ?= 0
TW_FRAMERATE ?= 30
TW_SUPPORT_INPUT_AIDL_HAPTICS_FQNAME ?= IVibrator/default

# Soong bool variables
SOONG_CONFIG_twrpGlobalVars_legacy_hw_disk_encryption := $(TARGET_LEGACY_HW_DISK_ENCRYPTION)
SOONG_CONFIG_twrpGlobalVars_target_enforce_ab_ota_partition_list := $(TARGET_ENFORCE_AB_OTA_PARTITION_LIST)
SOONG_CONFIG_twrpGlobalVars_supports_hw_fde := $(TARGET_HW_DISK_ENCRYPTION)
SOONG_CONFIG_twrpGlobalVars_supports_hw_fde_perf := $(TARGET_HW_DISK_ENCRYPTION_PERF)

SOONG_CONFIG_twrpGlobalVars_tw_delay_touch_init_ms := $(TW_DELAY_TOUCH_INIT_MS)
SOONG_CONFIG_twrpGlobalVars_tw_event_logging := $(TW_EVENT_LOGGING)
SOONG_CONFIG_twrpGlobalVars_tw_use_key_code_touch_sync := $(TW_USE_KEY_CODE_TOUCH_SYNC)
SOONG_CONFIG_twrpGlobalVars_tw_ozip_decrypt_key := $(subst ",, $(TW_OZIP_DECRYPT_KEY))
SOONG_CONFIG_twrpGlobalVars_tw_no_screen_blank := $(TW_NO_SCREEN_BLANK)
SOONG_CONFIG_twrpGlobalVars_tw_no_screen_timeout := $(TW_NO_SCREEN_TIMEOUT)
SOONG_CONFIG_twrpGlobalVars_tw_oem_build := $(TW_OEM_BUILD)
SOONG_CONFIG_twrpGlobalVars_tw_x_offset := $(TW_X_OFFSET)
SOONG_CONFIG_twrpGlobalVars_tw_y_offset := $(TW_Y_OFFSET)
SOONG_CONFIG_twrpGlobalVars_tw_w_offset := $(TW_W_OFFSET)
SOONG_CONFIG_twrpGlobalVars_tw_h_offset := $(TW_H_OFFSET)
SOONG_CONFIG_twrpGlobalVars_tw_framerate := $(TW_FRAMERATE)
SOONG_CONFIG_twrpGlobalVars_tw_round_screen := $(TW_ROUND_SCREEN)
SOONG_CONFIG_twrpGlobalVars_twrp_custom_keyboard := $(TWRP_CUSTOM_KEYBOARD)
SOONG_CONFIG_twrpGlobalVars_tw_support_input_1_2_haptics := $(TW_SUPPORT_INPUT_1_2_HAPTICS)
SOONG_CONFIG_twrpGlobalVars_tw_target_uses_qcom_bsp := $(TW_TARGET_USES_QCOM_BSP)
SOONG_CONFIG_twrpGlobalVars_tw_include_jpeg := $(TW_INCLUDE_JPEG)
SOONG_CONFIG_twrpGlobalVars_recovery_touchscreen_swap_xy := $(RECOVERY_TOUCHSCREEN_SWAP_XY)
SOONG_CONFIG_twrpGlobalVars_recovery_touchscreen_flip_x := $(RECOVERY_TOUCHSCREEN_FLIP_X)
SOONG_CONFIG_twrpGlobalVars_recovery_touchscreen_flip_y := $(RECOVERY_TOUCHSCREEN_FLIP_Y)
SOONG_CONFIG_twrpGlobalVars_recovery_graphics_force_use_linelength := $(RECOVERY_GRAPHICS_FORCE_USE_LINELENGTH)
SOONG_CONFIG_twrpGlobalVars_recovery_graphics_force_single_buffer := $(RECOVERY_GRAPHICS_FORCE_SINGLE_BUFFER)
SOONG_CONFIG_twrpGlobalVars_twrp_event_logging := $(TWRP_EVENT_LOGGING)
SOONG_CONFIG_twrpGlobalVars_target_recovery_overscan_percent := $(TARGET_RECOVERY_OVERSCAN_PERCENT)
SOONG_CONFIG_twrpGlobalVars_tw_screen_blank_on_boot := $(TW_SCREEN_BLANK_ON_BOOT)
SOONG_CONFIG_twrpGlobalVars_tw_fbiopan := $(TW_FBIOPAN)
SOONG_CONFIG_twrpGlobalVars_tw_ignore_major_axis_0 := $(TW_IGNORE_MAJOR_AXIS_0)
SOONG_CONFIG_twrpGlobalVars_tw_ignore_mt_position_0 := $(TW_IGNORE_MT_POSITION_0)
SOONG_CONFIG_twrpGlobalVars_tw_input_blacklist :=$(subst ",, $(TW_INPUT_BLACKLIST))
SOONG_CONFIG_twrpGlobalVars_tw_whitelist_input :=$(subst ",, $(TW_WHITELIST_INPUT))
SOONG_CONFIG_twrpGlobalVars_tw_haptics_tspdrv := $(TW_HAPTICS_TSPDRV)
SOONG_CONFIG_twrpGlobalVars_tw_exclude_nano := $(TW_EXCLUDE_NANO)
SOONG_CONFIG_twrpGlobalVars_tw_support_input_aidl_haptics := $(TW_SUPPORT_INPUT_AIDL_HAPTICS)
SOONG_CONFIG_twrpGlobalVars_tw_support_input_aidl_haptics_fqname := $(subst ",, $(TW_SUPPORT_INPUT_AIDL_HAPTICS_FQNAME))
SOONG_CONFIG_twrpGlobalVars_tw_support_input_aidl_haptics_fix_off := $(TW_SUPPORT_INPUT_AIDL_HAPTICS_FIX_OFF)
SOONG_CONFIG_twrpGlobalVars_tw_use_samsung_haptics := $(TW_USE_SAMSUNG_HAPTICS)
SOONG_CONFIG_twrpGlobalVars_tw_brightness_path := $(subst ",, $(TW_BRIGHTNESS_PATH))
SOONG_CONFIG_twrpGlobalVars_tw_max_brightness := $(TW_MAX_BRIGHTNESS)

ifneq ($(TARGET_CRYPTFS_HW_PATH),)
  SOONG_CONFIG_twrpGlobalVars_hw_fde_cryptfs_hw_header_lib_name := //$(TARGET_CRYPTFS_HW_PATH):libcryptfs_hw_headers
  SOONG_CONFIG_twrpGlobalVars_hw_fde_cryptfs_hw_shared_lib_name := //$(TARGET_CRYPTFS_HW_PATH):libcryptfs_hw
else
  SOONG_CONFIG_twrpGlobalVars_hw_fde_cryptfs_hw_header_lib_name := libcryptfs_hw_headers
  SOONG_CONFIG_twrpGlobalVars_hw_fde_cryptfs_hw_shared_lib_name := libcryptfs_hw
endif

# Set default values
TARGET_INIT_VENDOR_LIB ?= vendor_init

# Soong value variables
SOONG_CONFIG_twrpGlobalVars_target_init_vendor_lib := $(TARGET_INIT_VENDOR_LIB)
