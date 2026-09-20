/*
 * Copyright (C) 2020 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

#define LOG_TAG "bootcontrolhal"

#include "BootControl.h"

#include <android-base/file.h>
#include <android-base/unique_fd.h>
#include <bootloader_message/bootloader_message.h>
#include <cutils/properties.h>
#include <libboot_control/libboot_control.h>
#include <log/log.h>
#include <trusty/tipc.h>

#include "DevInfo.h"
#include "GptUtils.h"

namespace android {
namespace hardware {
namespace boot {
namespace V1_2 {
namespace implementation {

using android::bootable::GetMiscVirtualAbMergeStatus;
using android::bootable::InitMiscVirtualAbMessageIfNeeded;
using android::bootable::SetMiscVirtualAbMergeStatus;
using android::hardware::boot::V1_0::BoolResult;
using android::hardware::boot::V1_0::CommandResult;
using android::hardware::boot::V1_1::MergeStatus;

namespace {

// clang-format off

#define BOOT_A_PATH     "/dev/block/by-name/boot_a"
#define BOOT_B_PATH     "/dev/block/by-name/boot_b"
#define DEVINFO_PATH    "/dev/block/by-name/devinfo"

#define BLOW_AR_PATH    "/sys/kernel/boot_control/blow_ar"

// slot flags
#define AB_ATTR_PRIORITY_SHIFT      52
#define AB_ATTR_PRIORITY_MASK       (3UL << AB_ATTR_PRIORITY_SHIFT)
#define AB_ATTR_ACTIVE_SHIFT        54
#define AB_ATTR_ACTIVE              (1UL << AB_ATTR_ACTIVE_SHIFT)
#define AB_ATTR_RETRY_COUNT_SHIFT   (55)
#define AB_ATTR_RETRY_COUNT_MASK    (7UL << AB_ATTR_RETRY_COUNT_SHIFT)
#define AB_ATTR_SUCCESSFUL          (1UL << 58)
#define AB_ATTR_UNBOOTABLE          (1UL << 59)

#define AB_ATTR_MAX_PRIORITY        3UL
#define AB_ATTR_MAX_RETRY_COUNT     3UL

// clang-format on

static std::string getDevPath(uint32_t slot) {
    char real_path[PATH_MAX];

    const char *path = slot == 0 ? BOOT_A_PATH : BOOT_B_PATH;

    int ret = readlink(path, real_path, sizeof real_path);
    if (ret < 0) {
        ALOGE("readlink failed for boot device %s\n", strerror(errno));
        return std::string();
    }

    std::string dp(real_path);
    // extract /dev/sda.. part
    return dp.substr(0, sizeof "/dev/block/sdX" - 1);
}

static bool isSlotFlagSet(uint32_t slot, uint64_t flag) {
    std::string dev_path = getDevPath(slot);
    if (dev_path.empty()) {
        ALOGI("Could not get device path for slot %d\n", slot);
        return false;
    }

    GptUtils gpt(dev_path);
    if (gpt.Load()) {
        ALOGI("failed to load gpt data\n");
        return false;
    }

    gpt_entry *e = gpt.GetPartitionEntry(slot ? "boot_b" : "boot_a");
    if (e == nullptr) {
        ALOGI("failed to get gpt entry\n");
        return false;
    }

    return !!(e->attr & flag);
}

static bool setSlotFlag(uint32_t slot, uint64_t flag) {
    std::string dev_path = getDevPath(slot);
    if (dev_path.empty()) {
        ALOGI("Could not get device path for slot %d\n", slot);
        return false;
    }

    GptUtils gpt(dev_path);
    if (gpt.Load()) {
        ALOGI("failed to load gpt data\n");
        return false;
    }

    gpt_entry *e = gpt.GetPartitionEntry(slot ? "boot_b" : "boot_a");
    if (e == nullptr) {
        ALOGI("failed to get gpt entry\n");
        return false;
    }

    e->attr |= flag;
    gpt.Sync();

    return true;
}

static bool is_devinfo_valid;
static bool is_devinfo_initialized;
static std::mutex devinfo_lock;
static devinfo_t devinfo;

static bool isDevInfoValid() {
    const std::lock_guard<std::mutex> lock(devinfo_lock);

    if (is_devinfo_initialized) {
        return is_devinfo_valid;
    }

    is_devinfo_initialized = true;

    android::base::unique_fd fd(open(DEVINFO_PATH, O_RDONLY));
    android::base::ReadFully(fd, &devinfo, sizeof devinfo);

    if (devinfo.magic != DEVINFO_MAGIC) {
        return is_devinfo_valid;
    }

    uint32_t version = ((uint32_t)devinfo.ver_major << 16) | devinfo.ver_minor;
    // only version 3.3+ supports A/B data
    if (version >= 0x0003'0003) {
        is_devinfo_valid = true;
    }

    return is_devinfo_valid;
}

static bool DevInfoSync() {
    if (!isDevInfoValid()) {
        return false;
    }

    android::base::unique_fd fd(open(DEVINFO_PATH, O_WRONLY | O_DSYNC));
    return android::base::WriteFully(fd, &devinfo, sizeof devinfo);
}

static void DevInfoInitSlot(devinfo_ab_slot_data_t &slot_data) {
    slot_data.retry_count = AB_ATTR_MAX_RETRY_COUNT;
    slot_data.unbootable = 0;
    slot_data.successful = 0;
    slot_data.active = 1;
    slot_data.fastboot_ok = 0;
}

static int blow_otp_AR(bool secure) {
    static const char *dev_name = "/dev/trusty-ipc-dev0";
    static const char *otp_name = "com.android.trusty.otp_manager.tidl";
    int fd = 1, ret = 0;
    uint32_t cmd = secure? OTP_CMD_write_antirbk_secure_ap : OTP_CMD_write_antirbk_non_secure_ap;
    fd = tipc_connect(dev_name, otp_name);
    if (fd < 0) {
        ALOGI("Failed to connect to OTP_MGR ns TA - is it missing?\n");
        ret = -1;
        return ret;
    }

    struct otp_mgr_req_base req = {
        .command = cmd,
        .resp_payload_size = 0,
    };
    struct iovec iov[] = {
        {
            .iov_base = &req,
            .iov_len = sizeof(req),
        },
    };

    int rc = tipc_send(fd, iov, 1, NULL, 0);
    if (rc != sizeof(req)) {
        ALOGI("Send fail! %x\n", rc);
        return rc;
    }

    struct otp_mgr_rsp_base resp;
    rc = read(fd, &resp, sizeof(resp));
    if (rc < 0) {
        ALOGI("Read fail! %x\n", rc);
        return rc;
    }

    if (rc < sizeof(resp)) {
        ALOGI("Not enough data! %x\n", rc);
        return -EIO;
    }

    if (resp.command != (cmd | OTP_RESP_BIT)) {
        ALOGI("Wrong command! %x\n", resp.command);
        return -EINVAL;
    }

    if (resp.result != 0) {
        fprintf(stderr, "AR writing error! %x\n", resp.result);
        return -EINVAL;
    }

    tipc_close(fd);
    return 0;
}

static bool blowAR_zuma() {
    int ret = blow_otp_AR(true);
    if (ret) {
        ALOGI("Blow secure anti-rollback OTP failed");
        return false;
    }

    ret = blow_otp_AR(false);
    if (ret) {
        ALOGI("Blow non-secure anti-rollback OTP failed");
        return false;
    }

    return true;
}

static bool blowAR_gs101() {
    android::base::unique_fd fd(open(BLOW_AR_PATH, O_WRONLY | O_DSYNC));
    return android::base::WriteStringToFd("1", fd);
}

static bool blowAR() {
    char platform[PROPERTY_VALUE_MAX];
    property_get("ro.boot.hardware.platform", platform, "");

    if (std::string(platform) == "gs101") {
        return blowAR_gs101();
    } else if (std::string(platform) == "gs201" || std::string(platform) == "zuma") {
        return blowAR_zuma();
    }

    return true;
}

}  // namespace

// Methods from ::android::hardware::boot::V1_0::IBootControl follow.
Return<uint32_t> BootControl::getNumberSlots() {
    uint32_t slots = 0;

    if (access(BOOT_A_PATH, F_OK) == 0)
        slots++;

    if (access(BOOT_B_PATH, F_OK) == 0)
        slots++;

    return slots;
}

Return<uint32_t> BootControl::getCurrentSlot() {
    char suffix[PROPERTY_VALUE_MAX];
    property_get("ro.boot.slot_suffix", suffix, "_a");
    return std::string(suffix) == "_b" ? 1 : 0;
}

Return<void> BootControl::markBootSuccessful(markBootSuccessful_cb _hidl_cb) {
    if (getNumberSlots() == 0) {
        // no slots, just return true otherwise Android keeps trying
        _hidl_cb({true, ""});
        return Void();
    }

    bool ret;
    if (isDevInfoValid()) {
        auto const slot = getCurrentSlot();
        devinfo.ab_data.slots[slot].successful = 1;
        ret = DevInfoSync();
    } else {
        ret = setSlotFlag(getCurrentSlot(), AB_ATTR_SUCCESSFUL);
    }

    if (!ret) {
        _hidl_cb({false, "Failed to set successful flag"});
        return Void();
    }

    if (!blowAR()) {
        ALOGE("Failed to blow anti-rollback counter");
        // Ignore the error, since ABL will re-trigger it on reboot
    }

    _hidl_cb({true, ""});
    return Void();
}

Return<void> BootControl::setActiveBootSlot(uint32_t slot, setActiveBootSlot_cb _hidl_cb) {
    if (slot >= 2) {
        _hidl_cb({false, "Invalid slot"});
        return Void();
    }

    if (isDevInfoValid()) {
        auto &active_slot_data = devinfo.ab_data.slots[slot];
        auto &inactive_slot_data = devinfo.ab_data.slots[!slot];

        inactive_slot_data.active = 0;
        DevInfoInitSlot(active_slot_data);

        if (!DevInfoSync()) {
            _hidl_cb({false, "Could not update DevInfo data"});
            return Void();
        }
    } else {
        std::string dev_path = getDevPath(slot);
        if (dev_path.empty()) {
            _hidl_cb({false, "Could not get device path for slot"});
            return Void();
        }

        GptUtils gpt(dev_path);
        if (gpt.Load()) {
            _hidl_cb({false, "failed to load gpt data"});
            return Void();
        }

        gpt_entry *active_entry = gpt.GetPartitionEntry(slot == 0 ? "boot_a" : "boot_b");
        gpt_entry *inactive_entry = gpt.GetPartitionEntry(slot == 0 ? "boot_b" : "boot_a");
        if (active_entry == nullptr || inactive_entry == nullptr) {
            _hidl_cb({false, "failed to get entries for boot partitions"});
            return Void();
        }

        ALOGV("slot active attributes %lx\n", active_entry->attr);
        ALOGV("slot inactive attributes %lx\n", inactive_entry->attr);

        // update attributes for active and inactive
        inactive_entry->attr &= ~AB_ATTR_ACTIVE;
        active_entry->attr = AB_ATTR_ACTIVE | (AB_ATTR_MAX_PRIORITY << AB_ATTR_PRIORITY_SHIFT) |
                             (AB_ATTR_MAX_RETRY_COUNT << AB_ATTR_RETRY_COUNT_SHIFT);
    }

    char boot_dev[PROPERTY_VALUE_MAX];
    property_get("ro.boot.bootdevice", boot_dev, "");
    if (boot_dev[0] == '\0') {
        ALOGI("failed to get ro.boot.bootdevice. try ro.boot.boot_devices\n");
        property_get("ro.boot.boot_devices", boot_dev, "");
        if (boot_dev[0] == '\0') {
            _hidl_cb({false, "invalid ro.boot.bootdevice and ro.boot.boot_devices prop"});
            return Void();
        }
    }

    std::string boot_lun_path =
            std::string("/sys/devices/platform/") + boot_dev + "/pixel/boot_lun_enabled";
    int fd = open(boot_lun_path.c_str(), O_RDWR | O_DSYNC);
    if (fd < 0) {
        // Try old path for kernels < 5.4
        // TODO: remove once kernel 4.19 support is deprecated
        std::string boot_lun_path =
                std::string("/sys/devices/platform/") + boot_dev + "/attributes/boot_lun_enabled";
        fd = open(boot_lun_path.c_str(), O_RDWR | O_DSYNC);
        if (fd < 0) {
            _hidl_cb({false, "failed to open ufs attr boot_lun_enabled"});
            return Void();
        }
    }

    //
    // bBootLunEn
    // 0x1  => Boot LU A = enabled, Boot LU B = disable
    // 0x2  => Boot LU A = disable, Boot LU B = enabled
    //
    int ret = android::base::WriteStringToFd(slot == 0 ? "1" : "2", fd);
    close(fd);
    if (ret < 0) {
        _hidl_cb({false, "faied to write boot_lun_enabled attribute"});
        return Void();
    }

    _hidl_cb({true, ""});
    return Void();
}

Return<void> BootControl::setSlotAsUnbootable(uint32_t slot, setSlotAsUnbootable_cb _hidl_cb) {
    if (slot >= 2) {
        _hidl_cb({false, "Invalid slot"});
        return Void();
    }

    if (isDevInfoValid()) {
        auto &slot_data = devinfo.ab_data.slots[slot];
        slot_data.unbootable = 1;
        if (!DevInfoSync()) {
            _hidl_cb({false, "Could not update DevInfo data"});
            return Void();
        }
    } else {
        std::string dev_path = getDevPath(slot);
        if (dev_path.empty()) {
            _hidl_cb({false, "Could not get device path for slot"});
            return Void();
        }

        GptUtils gpt(dev_path);
        gpt.Load();

        gpt_entry *e = gpt.GetPartitionEntry(slot ? "boot_b" : "boot_a");
        e->attr |= AB_ATTR_UNBOOTABLE;

        gpt.Sync();
    }

    _hidl_cb({true, ""});
    return Void();
}

Return<::android::hardware::boot::V1_0::BoolResult> BootControl::isSlotBootable(uint32_t slot) {
    if (getNumberSlots() == 0)
        return BoolResult::FALSE;
    if (slot >= getNumberSlots())
        return BoolResult::INVALID_SLOT;

    bool unbootable;
    if (isDevInfoValid()) {
        auto &slot_data = devinfo.ab_data.slots[slot];
        unbootable = !!slot_data.unbootable;
    } else {
        unbootable = isSlotFlagSet(slot, AB_ATTR_UNBOOTABLE);
    }

    return unbootable ? BoolResult::FALSE : BoolResult::TRUE;
}

Return<::android::hardware::boot::V1_0::BoolResult> BootControl::isSlotMarkedSuccessful(
        uint32_t slot) {
    if (getNumberSlots() == 0) {
        // just return true so that we don't we another call trying to mark it as successful
        // when there is no slots
        return BoolResult::TRUE;
    }
    if (slot >= getNumberSlots())
        return BoolResult::INVALID_SLOT;

    bool successful;
    if (isDevInfoValid()) {
        auto &slot_data = devinfo.ab_data.slots[slot];
        successful = !!slot_data.successful;
    } else {
        successful = isSlotFlagSet(slot, AB_ATTR_SUCCESSFUL);
    }

    return successful ? BoolResult::TRUE : BoolResult::FALSE;
}

Return<void> BootControl::getSuffix(uint32_t slot, getSuffix_cb _hidl_cb) {
    _hidl_cb(slot == 0 ? "_a" : slot == 1 ? "_b" : "");
    return Void();
}

// Methods from ::android::hardware::boot::V1_1::IBootControl follow.
bool BootControl::Init() {
    return InitMiscVirtualAbMessageIfNeeded();
}

Return<bool> BootControl::setSnapshotMergeStatus(
        ::android::hardware::boot::V1_1::MergeStatus status) {
    return SetMiscVirtualAbMergeStatus(getCurrentSlot(), status);
}

Return<::android::hardware::boot::V1_1::MergeStatus> BootControl::getSnapshotMergeStatus() {
    MergeStatus status;
    if (!GetMiscVirtualAbMergeStatus(getCurrentSlot(), &status)) {
        return MergeStatus::UNKNOWN;
    }
    return status;
}

// Methods from ::android::hardware::boot::V1_2::IBootControl follow.
Return<uint32_t> BootControl::getActiveBootSlot() {
    if (getNumberSlots() == 0)
        return 0;

    if (isDevInfoValid())
        return devinfo.ab_data.slots[1].active ? 1 : 0;
    return isSlotFlagSet(1, AB_ATTR_ACTIVE) ? 1 : 0;
}

// Methods from ::android::hidl::base::V1_0::IBase follow.

IBootControl *HIDL_FETCH_IBootControl(const char * /* name */) {
    auto module = new BootControl();

    module->Init();

    return module;
}

}  // namespace implementation
}  // namespace V1_2
}  // namespace boot
}  // namespace hardware
}  // namespace android
