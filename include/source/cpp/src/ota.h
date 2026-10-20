// OTA package model: zip directory walk (libziparchive), payload header,
// DeltaArchiveManifest (update_metadata-protos), full-OTA verdict,
// pre-device check.

#pragma once

#include <cstdint>
#include <map>
#include <string>
#include <vector>

namespace hs {

struct OtaOp {
    int type = 99;
    uint64_t data_off = 0;
    uint64_t data_len = 0;
    // (start_block, num_blocks), block size from the manifest.
    std::vector<std::pair<uint64_t, uint64_t>> dst;
    // data_sha256_hash from the manifest (raw bytes, may be empty).
    std::string data_hash;
};

struct OtaPart {
    std::string name;
    std::vector<OtaOp> ops;
    bool has_old_info = false;
    uint64_t new_size = 0;
    std::string new_hash;  // raw bytes, may be empty
    uint64_t OutSize(uint64_t bs) const;
    uint64_t DlSize() const;
};

struct DynGroup {
    std::string name;
    uint64_t size = 0;
    std::vector<std::string> parts;
};

// Loads and parses the payload. Returns "" on success or an error string.
// Fills parts/groups/block_size/data_start. full_ota=false + reason when
// the package needs the stock path (delta ops, old_partition_info).
std::string ProbePayload(const std::string& zip_path,
                         std::vector<OtaPart>* parts,
                         std::vector<DynGroup>* groups, uint64_t* block_size,
                         uint64_t* data_start, bool* full_ota,
                         std::string* not_full_reason);

// pre-device from META-INF/com/android/metadata ("" if absent).
std::string OtaPreDevice(const std::string& zip_path);

}  // namespace hs
