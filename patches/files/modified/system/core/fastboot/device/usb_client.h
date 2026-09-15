/*
 * Copyright (C) 2018 The Android Open Source Project
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

#include <memory>

#include "usb.h"

#include "transport.h"

class ClientUsbTransport : public Transport {
  public:
    ClientUsbTransport();
    ~ClientUsbTransport() override = default;

    ssize_t Read(void* data, size_t len) override;
    ssize_t Write(const void* data, size_t len) override;
    int Close() override;
    int Reset() override;

  private:
    std::unique_ptr<usb_handle> handle_;

    // Set when a write proves the host is gone (error or 10s stall). While
    // set, writes fail fast instead of parking the daemon in io_getevents
    // once per swallowed response; cleared by the next completed read, which
    // proves a live host is polling again.
    bool write_broken_ = false;

    DISALLOW_COPY_AND_ASSIGN(ClientUsbTransport);
};
