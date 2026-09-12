/*
 * Copyright (C) 2021 The Android Open Source Project
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

namespace android {
namespace hardware {
namespace boot {
namespace V1_2 {
namespace implementation {

//
// definitions taken from ABL code
//

constexpr uint32_t DEVINFO_MAGIC = 0x49564544;
constexpr size_t DEVINFO_AB_SLOT_COUNT = 2;

struct devinfo_ab_slot_data_t {
    uint8_t retry_count;
    uint8_t unbootable : 1;
    uint8_t successful : 1;
    uint8_t active : 1;
    uint8_t fastboot_ok : 1;
    uint8_t : 4;
    uint8_t unused[2];
} __attribute__((packed));

typedef struct {
    devinfo_ab_slot_data_t slots[DEVINFO_AB_SLOT_COUNT];
} __attribute__((packed)) devinfo_ab_data_t;

struct devinfo_t {
    uint32_t magic;
    uint16_t ver_major;
    uint16_t ver_minor;
    uint8_t unused[40];
    devinfo_ab_data_t ab_data;
    uint8_t unused1[72];  // use remaining up to complete 128 bytes
} __attribute__((packed));

static_assert(sizeof(devinfo_t) == 128, "invalid devinfo struct size");

}  // namespace implementation
}  // namespace V1_2
}  // namespace boot
}  // namespace hardware
}  // namespace android
