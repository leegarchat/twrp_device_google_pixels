/*
 * Copyright (C) 2020 The Android Open Source Project
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

#define LOG_TAG "android.hardware.boot@1.2-service"

#include <android/hardware/boot/1.2/IBootControl.h>
#include <hidl/HidlTransportSupport.h>
#include <hidl/Status.h>
#include <log/log.h>

#include "BootControl.h"

using ::android::status_t;

using ::android::hardware::boot::V1_2::IBootControl;

using ::android::hardware::boot::V1_2::implementation::BootControl;
// using ::android::hardware::boot::implementation::BootControl;

int main(int /* argc */, char * /* argv */[]) {
    // This function must be called before you join to ensure the proper
    // number of threads are created. The threadpool will never exceed
    // size one because of this call.
    ::android::hardware::configureRpcThreadpool(1 /*threads*/, true /*willJoin*/);

    ::android::sp bootctrl = new BootControl();
    const status_t status = bootctrl->registerAsService();
    if (status != ::android::OK) {
        return 1;  // or handle error
    }

    // Adds this thread to the threadpool, resulting in one total
    // thread in the threadpool. We could also do other things, but
    // would have to specify 'false' to willJoin in configureRpcThreadpool.
    ::android::hardware::joinRpcThreadpool();
    return 1;  // joinRpcThreadpool should never return
}
