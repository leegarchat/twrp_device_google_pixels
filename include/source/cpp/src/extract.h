// Parallel payload writer: op-level tasks, per-op decode, pwrite scatter,
// read-back SHA256 validation.

#pragma once

#include <atomic>
#include <cstdint>
#include <functional>
#include <map>
#include <string>
#include <vector>

#include "ota.h"

namespace hs {

struct WriteTarget {
    std::string part;  // base name
    std::string node;  // /dev/block/mapper/<full>
    const OtaPart* manifest = nullptr;
};

struct WriteStats {
    uint64_t output_bytes = 0;
    uint64_t downloaded_bytes = 0;
};

// Write all (partition,op) tasks with `threads` workers. progress(done,out)
// is called (under lock) after every op. Returns false on first error.
bool WriteAll(const std::string& zip, uint64_t data_start, uint64_t bs,
              const std::vector<WriteTarget>& targets, unsigned threads,
              uint64_t total_out,
              const std::function<void(uint64_t, uint64_t)>& progress,
              std::string* err);

// Read back a node and hash it. Returns hex or "" on error.
std::string Sha256File(const std::string& path, uint64_t size);

}  // namespace hs
