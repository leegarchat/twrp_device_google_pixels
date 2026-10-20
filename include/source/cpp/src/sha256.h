// Minimal SHA-256 (public-domain style) to avoid extra deps in recovery.

#pragma once

#include <cstdint>
#include <cstring>

namespace hs {

class Sha256 {
   public:
    Sha256() { reset(); }
    void reset();
    void update(const uint8_t* data, size_t len);
    void final(uint8_t out[32]);

   private:
    void block(const uint8_t* p);
    uint32_t h_[8];
    uint64_t total_ = 0;
    uint8_t buf_[64];
    size_t buflen_ = 0;
};

}  // namespace hs
