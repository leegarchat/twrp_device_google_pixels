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

#pragma once

#include <android/hardware/boot/1.2/IBootControl.h>
#include <hidl/MQDescriptor.h>
#include <hidl/Status.h>

namespace android {
namespace hardware {
namespace boot {
namespace V1_2 {
namespace implementation {

using ::android::sp;
using ::android::hardware::hidl_array;
using ::android::hardware::hidl_memory;
using ::android::hardware::hidl_string;
using ::android::hardware::hidl_vec;
using ::android::hardware::Return;
using ::android::hardware::Void;

struct BootControl : public IBootControl {
    bool Init();

    // Methods from ::android::hardware::boot::V1_0::IBootControl follow.
    Return<uint32_t> getNumberSlots() override;
    Return<uint32_t> getCurrentSlot() override;
    Return<void> markBootSuccessful(markBootSuccessful_cb _hidl_cb) override;
    Return<void> setActiveBootSlot(uint32_t slot, setActiveBootSlot_cb _hidl_cb) override;
    Return<void> setSlotAsUnbootable(uint32_t slot, setSlotAsUnbootable_cb _hidl_cb) override;
    Return<::android::hardware::boot::V1_0::BoolResult> isSlotBootable(uint32_t slot) override;
    Return<::android::hardware::boot::V1_0::BoolResult> isSlotMarkedSuccessful(
            uint32_t slot) override;
    Return<void> getSuffix(uint32_t slot, getSuffix_cb _hidl_cb) override;

    // Methods from ::android::hardware::boot::V1_1::IBootControl follow.
    Return<bool> setSnapshotMergeStatus(
            ::android::hardware::boot::V1_1::MergeStatus status) override;
    Return<::android::hardware::boot::V1_1::MergeStatus> getSnapshotMergeStatus() override;

    // Methods from ::android::hardware::boot::V1_2::IBootControl follow.
    Return<uint32_t> getActiveBootSlot() override;

    // Methods from ::android::hidl::base::V1_0::IBase follow.
};

// FIXME: most likely delete, this is only for passthrough implementations
extern "C" IBootControl *HIDL_FETCH_IBootControl(const char *name);

enum otpmgr_command : uint32_t {
    OTP_REQ_SHIFT = 1,
    OTP_RESP_BIT = 1,
    OTP_CMD_write_antirbk_non_secure_ap = (7 << OTP_REQ_SHIFT),
    OTP_CMD_write_antirbk_secure_ap = (8 << OTP_REQ_SHIFT),
};

struct otp_mgr_req_base {
    uint32_t command;
    uint32_t resp_payload_size;
    uint8_t handle;
}__packed;

struct otp_mgr_rsp_base {
    uint32_t command;
    uint32_t resp_payload_size;
    int result;
}__packed;

}  // namespace implementation
}  // namespace V1_2
}  // namespace boot
}  // namespace hardware
}  // namespace android
