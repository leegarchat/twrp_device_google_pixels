//
// Copyright (C) 2015 The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

#include "update_engine/aosp/hardware_android.h"

#include <sys/types.h>

#include <memory>
#include <string>
#include <string_view>

#include <android/sysprop/GkiProperties.sysprop.h>
#include <android-base/properties.h>
#include <base/files/file_util.h>
#include <base/strings/string_number_conversions.h>
#include <base/strings/string_util.h>
#include <bootloader_message/bootloader_message.h>
#include <fstab/fstab.h>
#include <libavb/libavb.h>
#include <libavb_user/avb_ops_user.h>

#include "update_engine/common/error_code_utils.h"
#include "update_engine/common/hardware.h"
#include "update_engine/common/platform_constants.h"
#include "update_engine/common/utils.h"

#ifndef __ANDROID_RECOVERY__
#include <android/sysprop/OtaProperties.sysprop.h>
#endif

using android::base::GetBoolProperty;
using android::base::GetIntProperty;
using android::base::GetProperty;
using std::string;

namespace chromeos_update_engine {

namespace {

// Android properties that identify the hardware and potentially non-updatable
// parts of the bootloader (such as the bootloader version and the baseband
// version).
const char kPropProductManufacturer[] = "ro.product.manufacturer";
const char kPropBootHardwareSKU[] = "ro.boot.hardware.sku";
const char kPropBootRevision[] = "ro.boot.revision";
const char kPropBuildDateUTC[] = "ro.build.date.utc";

string GetPartitionBuildDate(const string& partition_name) {
  return android::base::GetProperty("ro." + partition_name + ".build.date.utc",
                                    "");
}

ErrorCode IsTimestampNewerLogged(const std::string& partition_name,
                                 const std::string& old_version,
                                 const std::string& new_version) {
  auto error_code = utils::IsTimestampNewer(old_version, new_version);
  if (error_code != ErrorCode::kSuccess) {
    LOG(WARNING) << "Timestamp check failed with "
                 << utils::ErrorCodeToString(error_code) << ": "
                 << partition_name << " Partition timestamp: " << old_version
                 << " Update timestamp: " << new_version;
  }
  return error_code;
}

void SetVbmetaDigestProp(const std::string& value) {
#ifndef __ANDROID_RECOVERY__
  if (!android::sysprop::OtaProperties::other_vbmeta_digest(value)) {
    LOG(WARNING) << "Failed to set other vbmeta digest to " << value;
  }
#endif
}

std::string CalculateVbmetaDigestForInactiveSlot() {
  AvbSlotVerifyData* avb_slot_data{};

  auto suffix = fs_mgr_get_other_slot_suffix();
  const char* requested_partitions[] = {nullptr};
  auto avb_ops = avb_ops_user_new();
  auto verify_result = avb_slot_verify(avb_ops,
                                       requested_partitions,
                                       suffix.c_str(),
                                       AVB_SLOT_VERIFY_FLAGS_NONE,
                                       AVB_HASHTREE_ERROR_MODE_EIO,
                                       &avb_slot_data);
  if (verify_result != AVB_SLOT_VERIFY_RESULT_OK) {
    LOG(WARNING) << "Failed to verify avb slot data: " << verify_result;
    return "";
  }

  uint8_t vbmeta_digest[AVB_SHA256_DIGEST_SIZE];
  avb_slot_verify_data_calculate_vbmeta_digest(
      avb_slot_data, AVB_DIGEST_TYPE_SHA256, vbmeta_digest);

  const std::string encoded_digest =
      base::HexEncode(vbmeta_digest, AVB_SHA256_DIGEST_SIZE);
  LOG(INFO) << "vbmeta digest for target slot: " << encoded_digest;
  return base::ToLowerASCII(encoded_digest);
}

}  // namespace

namespace hardware {

// Factory defined in hardware.h.
std::unique_ptr<HardwareInterface> CreateHardware() {
  return std::make_unique<HardwareAndroid>();
}

}  // namespace hardware

// In Android there are normally three kinds of builds: eng, userdebug and user.
// These builds target respectively a developer build, a debuggable version of
// the final product and the pristine final product the end user will run.
// Apart from the ro.build.type property name, they differ in the following
// properties that characterize the builds:
// * eng builds: ro.secure=0 and ro.debuggable=1
// * userdebug builds: ro.secure=1 and ro.debuggable=1
// * user builds: ro.secure=1 and ro.debuggable=0
//
// See IsOfficialBuild() and IsNormalMode() for the meaning of these options in
// Android.

bool HardwareAndroid::IsOfficialBuild() const {
  // We run an official build iff ro.secure == 1, because we expect the build to
  // behave like the end user product and check for updates. Note that while
  // developers are able to build "official builds" by just running "make user",
  // that will only result in a more restrictive environment. The important part
  // is that we don't produce and push "non-official" builds to the end user.
  //
  // In case of a non-bool value, we take the most restrictive option and
  // assume we are in an official-build.
  return GetBoolProperty("ro.secure", true);
}

bool HardwareAndroid::IsNormalBootMode() const {
  // We are running in "dev-mode" iff ro.debuggable == 1. In dev-mode the
  // update_engine will allow extra developers options, such as providing a
  // different update URL. In case of error, we assume the build is in
  // normal-mode.
  return !GetBoolProperty("ro.debuggable", false);
}

bool HardwareAndroid::AreDevFeaturesEnabled() const {
  return !IsNormalBootMode();
}

bool HardwareAndroid::IsOOBEEnabled() const {
  // No OOBE flow blocking updates for Android-based boards.
  return false;
}

bool HardwareAndroid::IsOOBEComplete(base::Time* out_time_of_oobe) const {
  LOG(WARNING) << "OOBE is not enabled but IsOOBEComplete() called.";
  if (out_time_of_oobe)
    *out_time_of_oobe = base::Time();
  return true;
}

string HardwareAndroid::GetHardwareClass() const {
  auto manufacturer = GetProperty(kPropProductManufacturer, "");
  auto sku = GetProperty(kPropBootHardwareSKU, "");
  auto revision = GetProperty(kPropBootRevision, "");

  return manufacturer + ":" + sku + ":" + revision;
}

string HardwareAndroid::GetDeviceRequisition() const {
  LOG(WARNING) << "STUB: Getting requisition is not supported.";
  return "";
}

int HardwareAndroid::GetMinKernelKeyVersion() const {
  LOG(WARNING) << "STUB: No Kernel key version is available.";
  return -1;
}

int HardwareAndroid::GetMinFirmwareKeyVersion() const {
  LOG(WARNING) << "STUB: No Firmware key version is available.";
  return -1;
}

int HardwareAndroid::GetMaxFirmwareKeyRollforward() const {
  LOG(WARNING) << "STUB: Getting firmware_max_rollforward is not supported.";
  return -1;
}

bool HardwareAndroid::SetMaxFirmwareKeyRollforward(
    int firmware_max_rollforward) {
  LOG(WARNING) << "STUB: Setting firmware_max_rollforward is not supported.";
  return false;
}

bool HardwareAndroid::SetMaxKernelKeyRollforward(int kernel_max_rollforward) {
  LOG(WARNING) << "STUB: Setting kernel_max_rollforward is not supported.";
  return false;
}

int HardwareAndroid::GetPowerwashCount() const {
  LOG(WARNING) << "STUB: Assuming no factory reset was performed.";
  return 0;
}

bool HardwareAndroid::SchedulePowerwash(bool save_rollback_data) {
  LOG(INFO) << "Scheduling a powerwash to BCB.";
  LOG_IF(WARNING, save_rollback_data) << "save_rollback_data was true but "
                                      << "isn't supported.";
  string err;
  if (!update_bootloader_message({"--wipe_data", "--reason=wipe_data_from_ota"},
                                 &err)) {
    LOG(ERROR) << "Failed to update bootloader message: " << err;
    return false;
  }
  return true;
}

bool HardwareAndroid::CancelPowerwash() {
  string err;
  if (!clear_bootloader_message(&err)) {
    LOG(ERROR) << "Failed to clear bootloader message: " << err;
    return false;
  }
  return true;
}

bool HardwareAndroid::GetNonVolatileDirectory(base::FilePath* path) const {
  base::FilePath local_path(constants::kNonVolatileDirectory);
  if (!base::DirectoryExists(local_path)) {
    LOG(ERROR) << "Non-volatile directory not found: " << local_path.value();
    return false;
  }
  *path = local_path;
  return true;
}

bool HardwareAndroid::GetPowerwashSafeDirectory(base::FilePath* path) const {
  // On Android, we don't have a directory persisted across powerwash.
  return false;
}

int64_t HardwareAndroid::GetBuildTimestamp() const {
  return GetIntProperty<int64_t>(kPropBuildDateUTC, 0);
}

// Returns true if the device runs an userdebug build, and explicitly allows OTA
// downgrade.
bool HardwareAndroid::AllowDowngrade() const {
  return GetBoolProperty("ro.ota.allow_downgrade", false) &&
         GetBoolProperty("ro.debuggable", false);
}

bool HardwareAndroid::GetFirstActiveOmahaPingSent() const {
  LOG(WARNING) << "STUB: Assuming first active omaha was never set.";
  return false;
}

bool HardwareAndroid::SetFirstActiveOmahaPingSent() {
  LOG(WARNING) << "STUB: Assuming first active omaha is set.";
  // We will set it true, so its failure doesn't cause escalation.
  return true;
}

void HardwareAndroid::SetWarmReset(bool warm_reset) {
  if constexpr (!constants::kIsRecovery) {
    constexpr char warm_reset_prop[] = "ota.warm_reset";
    if (!android::base::SetProperty(warm_reset_prop, warm_reset ? "1" : "0")) {
      LOG(WARNING) << "Failed to set prop " << warm_reset_prop;
    }
  }
}

void HardwareAndroid::SetVbmetaDigestForInactiveSlot(bool reset) {
  if constexpr (constants::kIsRecovery) {
    return;
  }

  if (android::base::GetProperty("ro.boot.avb_version", "").empty() &&
      android::base::GetProperty("ro.boot.vbmeta.avb_version", "").empty()) {
    LOG(INFO) << "Device doesn't use avb, skipping setting vbmeta digest";
    return;
  }

  if (reset) {
    SetVbmetaDigestProp("");
    return;
  }

  std::string digest = CalculateVbmetaDigestForInactiveSlot();
  if (digest.empty()) {
    LOG(WARNING) << "Failed to calculate the vbmeta digest for the other slot";
    return;
  }
  SetVbmetaDigestProp(digest);
}

string HardwareAndroid::GetVersionForLogging(
    const string& partition_name) const {
  if (partition_name == "boot") {
    // ro.bootimage.build.date.utc
    return GetPartitionBuildDate("bootimage");
  }
  return GetPartitionBuildDate(partition_name);
}

ErrorCode HardwareAndroid::IsPartitionUpdateValid(
    const string& partition_name, const string& new_version) const {
  if (partition_name == "boot") {
    const auto old_version = GetPartitionBuildDate("bootimage");
    auto error_code =
        IsTimestampNewerLogged(partition_name, old_version, new_version);
    if (error_code == ErrorCode::kPayloadTimestampError) {
      bool prevent_downgrade =
          android::sysprop::GkiProperties::prevent_downgrade_version().value_or(
              false);
      if (!prevent_downgrade) {
        LOG(WARNING) << "Downgrade of boot image is detected, but permitting "
                        "update because device does not prevent boot image "
                        "downgrade";
        // If prevent_downgrade_version sysprop is not explicitly set, permit
        // downgrade in boot image version.
        // Even though error_code is overridden here, always call
        // IsTimestampNewerLogged to produce log messages.
        error_code = ErrorCode::kSuccess;
      }
    }
    return error_code;
  }

  const auto old_version = GetPartitionBuildDate(partition_name);
  // TODO(zhangkelvin)  for some partitions, missing a current timestamp should
  // be an error, e.g. system, vendor, product etc.
  auto error_code =
      IsTimestampNewerLogged(partition_name, old_version, new_version);
  return error_code;
}

// Mount options for non-system partitions. This option causes selinux treat
// every file in the mounted filesystem as having the 'postinstall_file'
// context, regardless of what the filesystem itself records. See "SELinux
// User's and Administrator's Guide" for more information on this option.
constexpr const char* kDefaultPostinstallMountOptions =
    "context=u:object_r:postinstall_file:s0";

// Mount options for system partitions. This option causes selinux to use the
// 'postinstall_file' context as a fallback if there are no other selinux
// contexts associated with the file in the mounted partition. See "SELinux
// User's and Administrator's Guide" for more information on this option.
constexpr const char* kSystemPostinstallMountOptions =
    "defcontext=u:object_r:postinstall_file:s0";

// Name of the system-partition
constexpr std::string_view kSystemPartitionName = "system";

const char* HardwareAndroid::GetPartitionMountOptions(
    const std::string& partition_name) const {
  if (partition_name == kSystemPartitionName) {
    return kSystemPostinstallMountOptions;
  } else {
    return kDefaultPostinstallMountOptions;
  }
}

}  // namespace chromeos_update_engine
