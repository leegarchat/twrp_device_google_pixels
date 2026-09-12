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

#define LOG_TAG "bootcontrolhal"

#include "GptUtils.h"

#include <android-base/file.h>
#include <errno.h>
#include <linux/fs.h>
#include <log/log.h>
#include <zlib.h>

namespace android {
namespace hardware {
namespace boot {
namespace V1_2 {
namespace implementation {

namespace {

static int ValidateGptHeader(gpt_header *gpt) {
    if (gpt->signature != GPT_SIGNATURE) {
        ALOGE("invalid gpt signature 0x%lx\n", gpt->signature);
        return -1;
    }

    if (gpt->header_size != sizeof(gpt_header)) {
        ALOGE("invalid gpt header size %u\n", gpt->header_size);
        return -1;
    }

    if (gpt->entry_size != sizeof(gpt_entry)) {
        ALOGE("invalid gpt entry size %u\n", gpt->entry_size);
        return -1;
    }

    return 0;
}

}  // namespace

GptUtils::GptUtils(const std::string dev_path) : dev_path(dev_path), fd(0) {}

int GptUtils::Load(void) {
    fd = open(dev_path.c_str(), O_RDWR);
    if (fd < 0) {
        ALOGE("failed to open block dev %s, %d\n", dev_path.c_str(), errno);
        return -1;
    }

    int ret = ioctl(fd, BLKSSZGET, &block_size);
    if (ret < 0) {
        ALOGE("failed to get block size %d\n", errno);
        return -1;
    }

    // read primary header
    lseek64(fd, block_size, SEEK_SET);
    ret = read(fd, &gpt_primary, sizeof gpt_primary);
    if (ret < 0) {
        ALOGE("failed to read gpt primary header %d\n", errno);
        return -1;
    }

    if (ValidateGptHeader(&gpt_primary)) {
        ALOGE("error validating gpt header\n");
        return -1;
    }

    // read partition entries
    entry_array.resize(gpt_primary.entry_count);
    uint32_t entries_size = gpt_primary.entry_size * gpt_primary.entry_count;
    lseek64(fd, block_size * gpt_primary.start_lba, SEEK_SET);
    ret = read(fd, entry_array.data(), entries_size);
    if (ret < 0) {
        ALOGE("failed to read gpt partition entries %d\n", errno);
        return -1;
    }

    // read gpt back header
    lseek64(fd, block_size * gpt_primary.backup_lba, SEEK_SET);
    ret = read(fd, &gpt_backup, sizeof gpt_backup);
    if (ret < 0) {
        ALOGE("failed to read gpt backup header %d\n", errno);
        return -1;
    }

    if (ValidateGptHeader(&gpt_backup)) {
        ALOGW("error validating gpt backup\n");  // just warn about it, not fail
    }

    // Create map <partition name, gpt_entry pointer>
    auto get_name = [](const uint16_t *efi_name) {
        char name[37] = {};
        for (int i = 0; efi_name[i] && i < sizeof name - 1; ++i) name[i] = efi_name[i];
        return std::string(name);
    };

    for (auto const &e : entry_array) {
        if (e.name[0] == 0)
            break;  // stop at the first partition with no name
        std::string s = get_name(e.name);
        entries[s] = const_cast<gpt_entry *>(&e);
    }

    return 0;
}

gpt_entry *GptUtils::GetPartitionEntry(std::string name) {
    return entries.find(name) != entries.end() ? entries[name] : nullptr;
}

int GptUtils::Sync(void) {
    if (!fd)
        return -1;

    // calculate crc and check if we need to update gpt
    gpt_primary.entries_crc32 = crc32(0, reinterpret_cast<uint8_t *>(entry_array.data()),
                                      entry_array.size() * sizeof(gpt_entry));

    // save old crc
    uint32_t crc = gpt_primary.crc32;
    gpt_primary.crc32 = 0;

    gpt_primary.crc32 = crc32(0, reinterpret_cast<uint8_t *>(&gpt_primary), sizeof gpt_primary);
    if (crc == gpt_primary.crc32)
        return 0;  // nothing to do (no changes)

    ALOGI("updating GPT\n");

    lseek64(fd, block_size * gpt_primary.current_lba, SEEK_SET);
    int ret = write(fd, &gpt_primary, sizeof gpt_primary);
    if (ret < 0) {
        ALOGE("failed to write gpt primary header %d\n", errno);
        return -1;
    }

    lseek64(fd, block_size * gpt_primary.start_lba, SEEK_SET);
    ret = write(fd, entry_array.data(), entry_array.size() * sizeof(gpt_entry));
    if (ret < 0) {
        ALOGE("failed to write gpt partition entries %d\n", errno);
        return -1;
    }

    // update GPT backup entries and backup
    lseek64(fd, block_size * gpt_backup.start_lba, SEEK_SET);
    ret = write(fd, entry_array.data(), entry_array.size() * sizeof(gpt_entry));
    if (ret < 0) {
        ALOGE("failed to write gpt backup partition entries %d\n", errno);
        return -1;
    }

    gpt_backup.entries_crc32 = gpt_primary.entries_crc32;
    gpt_backup.crc32 = 0;
    gpt_backup.crc32 = crc32(0, reinterpret_cast<uint8_t *>(&gpt_backup), sizeof gpt_backup);
    lseek64(fd, block_size * gpt_primary.backup_lba, SEEK_SET);
    ret = write(fd, &gpt_backup, sizeof gpt_backup);
    if (ret < 0) {
        ALOGE("failed to write gpt backup header %d\n", errno);
        return -1;
    }

    fsync(fd);

    return 0;
}

GptUtils::~GptUtils() {
    if (fd) {
        Sync();
        close(fd);
    }
}

}  // namespace implementation
}  // namespace V1_2
}  // namespace boot
}  // namespace hardware
}  // namespace android
