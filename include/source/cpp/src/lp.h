// Own LP surgery (no siw): unmount+unmap everything, remove/create via
// liblp MetadataBuilder, map via libfs_mgr. Mirrors the proven engine
// flow, minus the fork/exec crutches.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

#include "plan.h"

namespace hs {

constexpr const char* kSuperBlock = "/dev/block/by-name/super";

// Unmount (deepest first) + unmap every dm node stacked on super.
// Raw LP writes are refused while any mapping holds the device.
// Returns unmapped names; hard error if anything stays busy.
bool UnmountUnmapAll(const LiveState& live, std::vector<std::string>* gone,
                     std::string* err);

// Remove one LP entry from EVERY metadata slot holding it (mirrored
// tables must not diverge). Tolerates absence (idempotent rebuilds).
bool RemovePartitionLp(const std::string& full, std::string* err);

// Create one LP entry with an exact size in the given group.
// Tries metadata slots in order, first success wins.
bool CreatePartitionLp(const std::string& full, const std::string& group,
                       uint64_t size, std::string* err);

// Map for writing (force-writable). metadata_slot selects the LP table
// the entry is resolved from (lptools parity: never rely on lib defaults).
// Returns the /dev/block/mapper path in out_path. Retries once after
// unmapping a stale idle node; a live (ACTIVE) node is a hard error.
bool MapWritable(const std::string& full, uint32_t metadata_slot,
                 std::string* out_path, std::string* err);

// Which metadata slot's table currently holds `full` (tries 0|1).
// Empty = absent everywhere (fresh create) or unreadable.
std::optional<uint32_t> ResolveMetadataSlot(const std::string& full);

}  // namespace hs
