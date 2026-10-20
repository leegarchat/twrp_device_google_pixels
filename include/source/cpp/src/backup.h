// Critical-partition backup: dump live dm nodes to /tmp before the
// first write, restore them on batch failure. Weights 2+4 only.

#pragma once

#include <cstdint>
#include <string>

namespace hs {

bool DumpNode(const std::string& node, const std::string& file, uint64_t size,
              std::string* err);
bool RestoreNode(const std::string& file, const std::string& node,
                 std::string* err);
bool FsyncNode(const std::string& node, std::string* err);
// Readback (peridot --readback parity): drop the page cache for the node
// (BLKFLSBUF) and hash what is actually on storage. Returns hex or "".
std::string ReadbackCheck(const std::string& node, uint64_t size,
                          std::string* err);

}  // namespace hs
