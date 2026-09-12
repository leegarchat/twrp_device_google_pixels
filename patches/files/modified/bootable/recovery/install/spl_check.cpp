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

#include "install/spl_check.h"

bool ViolatesSPLDowngrade(const build::tools::releasetools::OtaMetadata& /* metadata */,
                          std::string_view /* current_spl */) {
  // OrangeFox: SPL downgrade check disabled — allow any OTA
  LOG(INFO) << "ViolatesSPLDowngrade: check disabled, permitting OTA";
  return false;
}

bool ViolatesSPLDowngrade(ZipArchiveHandle zip, std::string_view current_spl) {
  static constexpr auto&& OTA_OTA_METADATA = "META-INF/com/android/metadata.pb";
  ZipEntry64 metadata_entry;
  if (FindEntry(zip, OTA_OTA_METADATA, &metadata_entry) != 0) {
    LOG(WARNING) << "Failed to find " << OTA_OTA_METADATA
                 << " treating this as non-spl-downgrade, permit OTA install. If device bricks "
                    "after installing, check kernel log to see if /data failed to decrypt";
    return false;
  }
  const auto metadata_entry_length = metadata_entry.uncompressed_length;
  if (metadata_entry_length > std::numeric_limits<size_t>::max()) {
    LOG(ERROR) << "Failed to extract " << OTA_OTA_METADATA
               << " because's uncompressed size exceeds size of address space. "
               << metadata_entry_length;
    return false;
  }
  std::vector<uint8_t> ota_metadata(metadata_entry_length);
  int32_t err = ExtractToMemory(zip, &metadata_entry, ota_metadata.data(), metadata_entry_length);
  if (err != 0) {
    LOG(ERROR) << "Failed to extract " << OTA_OTA_METADATA << ": " << ErrorCodeString(err);
    return false;
  }
  using build::tools::releasetools::OtaMetadata;
  OtaMetadata metadata;
  if (!metadata.ParseFromArray(ota_metadata.data(), ota_metadata.size())) {
    LOG(ERROR) << "Failed to parse ota_medata";
    return false;
  }
  return ViolatesSPLDowngrade(metadata, current_spl);
}
