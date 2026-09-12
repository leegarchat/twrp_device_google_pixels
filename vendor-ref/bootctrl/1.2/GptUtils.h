/*
 * Copyright (C) 2019 The Android Open Source Project
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

#include <map>
#include <string>
#include <vector>

namespace android {
namespace hardware {
namespace boot {
namespace V1_2 {
namespace implementation {

#define GPT_SIGNATURE 0x5452415020494645UL

typedef struct {
    uint8_t type_guid[16];
    uint8_t guid[16];
    uint64_t first_lba;
    uint64_t last_lba;
    uint64_t attr;
    uint16_t name[36];
} __attribute__((packed)) gpt_entry;

typedef struct {
    uint64_t signature;
    uint32_t revision;
    uint32_t header_size;
    uint32_t crc32;
    uint32_t reserved;
    uint64_t current_lba;
    uint64_t backup_lba;
    uint64_t first_usable_lba;
    uint64_t last_usable_lba;
    uint8_t disk_guid[16];
    uint64_t start_lba;
    uint32_t entry_count;
    uint32_t entry_size;
    uint32_t entries_crc32;
} __attribute__((packed)) gpt_header;

class GptUtils {
  public:
    GptUtils(const std::string dev_path);
    int Load(void);
    gpt_entry *GetPartitionEntry(std::string name);
    int Sync(void);
    ~GptUtils();

  private:
    std::string dev_path;
    int fd;
    uint32_t block_size;
    gpt_header gpt_primary;
    gpt_header gpt_backup;
    std::vector<gpt_entry> entry_array;
    std::map<std::string, gpt_entry *> entries;
};

}  // namespace implementation
}  // namespace V1_2
}  // namespace boot
}  // namespace hardware
}  // namespace android
