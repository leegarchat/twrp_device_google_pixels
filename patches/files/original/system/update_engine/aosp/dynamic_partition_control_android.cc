//
// Copyright (C) 2018 The Android Open Source Project
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

#include "update_engine/aosp/dynamic_partition_control_android.h"

#include <algorithm>
#include <chrono>  // NOLINT(build/c++11) - using libsnapshot / liblp API
#include <cstdint>
#include <map>
#include <memory>
#include <set>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include <android-base/properties.h>
#include <android-base/strings.h>
#include <base/files/file_util.h>
#include <base/logging.h>
#include <base/strings/string_util.h>
#include <base/strings/stringprintf.h>
#include <bootloader_message/bootloader_message.h>
#include <fs_mgr.h>
#include <fs_mgr_dm_linear.h>
#include <fs_mgr_overlayfs.h>
#include <libavb/libavb.h>
#include <libdm/dm.h>
#include <liblp/liblp.h>
#include <libsnapshot/cow_writer.h>
#include <libsnapshot/snapshot.h>
#include <libsnapshot/snapshot_stub.h>

#include "update_engine/aosp/cleanup_previous_update_action.h"
#include "update_engine/aosp/dynamic_partition_utils.h"
#include "update_engine/common/boot_control_interface.h"
#include "update_engine/common/dynamic_partition_control_interface.h"
#include "update_engine/common/error_code.h"
#include "update_engine/common/platform_constants.h"
#include "update_engine/common/utils.h"
#include "update_engine/payload_consumer/cow_writer_file_descriptor.h"
#include "update_engine/payload_consumer/delta_performer.h"

using android::base::GetBoolProperty;
using android::base::GetProperty;
using android::base::Join;
using android::dm::DeviceMapper;
using android::dm::DmDeviceState;
using android::fs_mgr::CreateLogicalPartition;
using android::fs_mgr::CreateLogicalPartitionParams;
using android::fs_mgr::DestroyLogicalPartition;
using android::fs_mgr::Fstab;
using android::fs_mgr::MetadataBuilder;
using android::fs_mgr::Partition;
using android::fs_mgr::PartitionOpener;
using android::fs_mgr::SlotSuffixForSlotNumber;
using android::snapshot::OptimizeSourceCopyOperation;
using android::snapshot::Return;
using android::snapshot::SnapshotManager;
using android::snapshot::SnapshotManagerStub;
using android::snapshot::UpdateState;
using base::StringPrintf;

namespace chromeos_update_engine {

constexpr char kUseDynamicPartitions[] = "ro.boot.dynamic_partitions";
constexpr char kRetrfoitDynamicPartitions[] =
    "ro.boot.dynamic_partitions_retrofit";
constexpr char kVirtualAbEnabled[] = "ro.virtual_ab.enabled";
constexpr char kVirtualAbRetrofit[] = "ro.virtual_ab.retrofit";
constexpr char kVirtualAbCompressionEnabled[] =
    "ro.virtual_ab.compression.enabled";
constexpr auto&& kVirtualAbCompressionXorEnabled =
    "ro.virtual_ab.compression.xor.enabled";
constexpr char kVirtualAbUserspaceSnapshotsEnabled[] =
    "ro.virtual_ab.userspace.snapshots.enabled";

// Currently, android doesn't have a retrofit prop for VAB Compression. However,
// struct FeatureFlag forces us to determine if a feature is 'retrofit'. So this
// is here just to simplify code. Replace it with real retrofit prop name once
// there is one.
constexpr char kVirtualAbCompressionRetrofit[] = "";
constexpr char kPostinstallFstabPrefix[] = "ro.postinstall.fstab.prefix";
// Map timeout for dynamic partitions.
constexpr std::chrono::milliseconds kMapTimeout{1000};
// Map timeout for dynamic partitions with snapshots. Since several devices
// needs to be mapped, this timeout is longer than |kMapTimeout|.
constexpr std::chrono::milliseconds kMapSnapshotTimeout{10000};

DynamicPartitionControlAndroid::~DynamicPartitionControlAndroid() {
  UnmapAllPartitions();
  metadata_device_.reset();
}

static FeatureFlag GetFeatureFlag(const char* enable_prop,
                                  const char* retrofit_prop) {
  // Default retrofit to false if retrofit_prop is empty.
  bool retrofit = retrofit_prop && retrofit_prop[0] != '\0' &&
                  GetBoolProperty(retrofit_prop, false);
  bool enabled = GetBoolProperty(enable_prop, false);
  if (retrofit && !enabled) {
    LOG(ERROR) << retrofit_prop << " is true but " << enable_prop
               << " is not. These sysprops are inconsistent. Assume that "
               << enable_prop << " is true from now on.";
  }
  if (retrofit) {
    return FeatureFlag(FeatureFlag::Value::RETROFIT);
  }
  if (enabled) {
    return FeatureFlag(FeatureFlag::Value::LAUNCH);
  }
  return FeatureFlag(FeatureFlag::Value::NONE);
}

DynamicPartitionControlAndroid::DynamicPartitionControlAndroid(
    uint32_t source_slot)
    : dynamic_partitions_(
          GetFeatureFlag(kUseDynamicPartitions, kRetrfoitDynamicPartitions)),
      virtual_ab_(GetFeatureFlag(kVirtualAbEnabled, kVirtualAbRetrofit)),
      virtual_ab_compression_(GetFeatureFlag(kVirtualAbCompressionEnabled,
                                             kVirtualAbCompressionRetrofit)),
      virtual_ab_compression_xor_(
          GetFeatureFlag(kVirtualAbCompressionXorEnabled, "")),
      virtual_ab_userspace_snapshots_(
          GetFeatureFlag(kVirtualAbUserspaceSnapshotsEnabled, nullptr)),
      source_slot_(source_slot) {
  if (GetVirtualAbFeatureFlag().IsEnabled()) {
    snapshot_ = SnapshotManager::New();
  } else {
    snapshot_ = SnapshotManagerStub::New();
  }
  CHECK(snapshot_ != nullptr) << "Cannot initialize SnapshotManager.";
}

FeatureFlag DynamicPartitionControlAndroid::GetDynamicPartitionsFeatureFlag() {
  return dynamic_partitions_;
}

FeatureFlag DynamicPartitionControlAndroid::GetVirtualAbFeatureFlag() {
  return virtual_ab_;
}

FeatureFlag
DynamicPartitionControlAndroid::GetVirtualAbCompressionFeatureFlag() {
  if constexpr (constants::kIsRecovery) {
    // Don't attempt VABC in recovery
    return FeatureFlag(FeatureFlag::Value::NONE);
  }
  return virtual_ab_compression_;
}

FeatureFlag
DynamicPartitionControlAndroid::GetVirtualAbCompressionXorFeatureFlag() {
  return virtual_ab_compression_xor_;
}

bool DynamicPartitionControlAndroid::OptimizeOperation(
    const std::string& partition_name,
    const InstallOperation& operation,
    InstallOperation* optimized) {
  switch (operation.type()) {
    case InstallOperation::SOURCE_COPY:
      return target_supports_snapshot_ &&
             GetVirtualAbFeatureFlag().IsEnabled() &&
             mapped_devices_.count(partition_name +
                                   SlotSuffixForSlotNumber(target_slot_)) > 0 &&
             OptimizeSourceCopyOperation(operation, optimized);
      break;
    default:
      break;
  }
  return false;
}

bool DynamicPartitionControlAndroid::MapPartitionInternal(
    const std::string& super_device,
    const std::string& target_partition_name,
    uint32_t slot,
    bool force_writable,
    std::string* path) {
  CreateLogicalPartitionParams params = {
      .block_device = super_device,
      .metadata_slot = slot,
      .partition_name = target_partition_name,
      .force_writable = force_writable,
  };
  bool success = false;
  if (GetVirtualAbFeatureFlag().IsEnabled() && target_supports_snapshot_ &&
      force_writable && ExpectMetadataMounted()) {
    // Only target partitions are mapped with force_writable. On Virtual
    // A/B devices, target partitions may overlap with source partitions, so
    // they must be mapped with snapshot.
    // One exception is when /metadata is not mounted. Fallback to
    // CreateLogicalPartition as snapshots are not created in the first place.
    params.timeout_ms = kMapSnapshotTimeout;
    success = snapshot_->MapUpdateSnapshot(params, path);
  } else {
    params.timeout_ms = kMapTimeout;
    success = CreateLogicalPartition(params, path);
  }

  if (!success) {
    LOG(ERROR) << "Cannot map " << target_partition_name << " in "
               << super_device << " on device mapper.";
    return false;
  }
  LOG(INFO) << "Succesfully mapped " << target_partition_name
            << " to device mapper (force_writable = " << force_writable
            << "); device path at " << *path;
  mapped_devices_.insert(target_partition_name);
  return true;
}

bool DynamicPartitionControlAndroid::MapPartitionOnDeviceMapper(
    const std::string& super_device,
    const std::string& target_partition_name,
    uint32_t slot,
    bool force_writable,
    std::string* path) {
  DmDeviceState state = GetState(target_partition_name);
  if (state == DmDeviceState::ACTIVE) {
    if (mapped_devices_.find(target_partition_name) != mapped_devices_.end()) {
      if (GetDmDevicePathByName(target_partition_name, path)) {
        LOG(INFO) << target_partition_name
                  << " is mapped on device mapper: " << *path;
        return true;
      }
      LOG(ERROR) << target_partition_name << " is mapped but path is unknown.";
      return false;
    }
    // If target_partition_name is not in mapped_devices_ but state is ACTIVE,
    // the device might be mapped incorrectly before. Attempt to unmap it.
    // Note that for source partitions, if GetState() == ACTIVE, callers (e.g.
    // BootControlAndroid) should not call MapPartitionOnDeviceMapper, but
    // should directly call GetDmDevicePathByName.
    if (!UnmapPartitionOnDeviceMapper(target_partition_name)) {
      LOG(ERROR) << target_partition_name
                 << " is mapped before the update, and it cannot be unmapped.";
      return false;
    }
    state = GetState(target_partition_name);
    if (state != DmDeviceState::INVALID) {
      LOG(ERROR) << target_partition_name << " is unmapped but state is "
                 << static_cast<std::underlying_type_t<DmDeviceState>>(state);
      return false;
    }
  }
  if (state == DmDeviceState::INVALID) {
    return MapPartitionInternal(
        super_device, target_partition_name, slot, force_writable, path);
  }

  LOG(ERROR) << target_partition_name
             << " is mapped on device mapper but state is unknown: "
             << static_cast<std::underlying_type_t<DmDeviceState>>(state);
  return false;
}

bool DynamicPartitionControlAndroid::UnmapPartitionOnDeviceMapper(
    const std::string& target_partition_name) {
  if (DeviceMapper::Instance().GetState(target_partition_name) !=
      DmDeviceState::INVALID) {
    // Partitions at target slot on non-Virtual A/B devices are mapped as
    // dm-linear. Also, on Virtual A/B devices, system_other may be mapped for
    // preopt apps as dm-linear.
    // Call DestroyLogicalPartition to handle these cases.
    bool success = DestroyLogicalPartition(target_partition_name);

    // On a Virtual A/B device, |target_partition_name| may be a leftover from
    // a paused update. Clean up any underlying devices.
    if (ExpectMetadataMounted()) {
      success &= snapshot_->UnmapUpdateSnapshot(target_partition_name);
    } else {
      LOG(INFO) << "Skip UnmapUpdateSnapshot(" << target_partition_name
                << ") because metadata is not mounted";
    }

    if (!success) {
      LOG(ERROR) << "Cannot unmap " << target_partition_name
                 << " from device mapper.";
      return false;
    }
    LOG(INFO) << "Successfully unmapped " << target_partition_name
              << " from device mapper.";
  }
  mapped_devices_.erase(target_partition_name);
  return true;
}

bool DynamicPartitionControlAndroid::UnmapAllPartitions() {
  snapshot_->UnmapAllSnapshots();
  if (mapped_devices_.empty()) {
    return false;
  }
  // UnmapPartitionOnDeviceMapper removes objects from mapped_devices_, hence
  // a copy is needed for the loop.
  std::set<std::string> mapped = mapped_devices_;
  LOG(INFO) << "Destroying [" << Join(mapped, ", ") << "] from device mapper";
  for (const auto& partition_name : mapped) {
    ignore_result(UnmapPartitionOnDeviceMapper(partition_name));
  }
  return true;
}

void DynamicPartitionControlAndroid::Cleanup() {
  UnmapAllPartitions();
  metadata_device_.reset();
  if (GetVirtualAbFeatureFlag().IsEnabled()) {
    snapshot_ = SnapshotManager::New();
  } else {
    snapshot_ = SnapshotManagerStub::New();
  }
  CHECK(snapshot_ != nullptr) << "Cannot initialize SnapshotManager.";
}

bool DynamicPartitionControlAndroid::DeviceExists(const std::string& path) {
  return base::PathExists(base::FilePath(path));
}

android::dm::DmDeviceState DynamicPartitionControlAndroid::GetState(
    const std::string& name) {
  return DeviceMapper::Instance().GetState(name);
}

bool DynamicPartitionControlAndroid::GetDmDevicePathByName(
    const std::string& name, std::string* path) {
  return DeviceMapper::Instance().GetDmDevicePathByName(name, path);
}

std::unique_ptr<MetadataBuilder>
DynamicPartitionControlAndroid::LoadMetadataBuilder(
    const std::string& super_device, uint32_t slot) {
  auto builder = MetadataBuilder::New(PartitionOpener(), super_device, slot);
  if (builder == nullptr) {
    LOG(WARNING) << "No metadata slot " << BootControlInterface::SlotName(slot)
                 << " in " << super_device;
    return nullptr;
  }
  LOG(INFO) << "Loaded metadata from slot "
            << BootControlInterface::SlotName(slot) << " in " << super_device;
  return builder;
}

std::unique_ptr<MetadataBuilder>
DynamicPartitionControlAndroid::LoadMetadataBuilder(
    const std::string& super_device,
    uint32_t source_slot,
    uint32_t target_slot) {
  bool always_keep_source_slot = !target_supports_snapshot_;
  auto builder = MetadataBuilder::NewForUpdate(PartitionOpener(),
                                               super_device,
                                               source_slot,
                                               target_slot,
                                               always_keep_source_slot);
  if (builder == nullptr) {
    LOG(WARNING) << "No metadata slot "
                 << BootControlInterface::SlotName(source_slot) << " in "
                 << super_device;
    return nullptr;
  }
  LOG(INFO) << "Created metadata for new update from slot "
            << BootControlInterface::SlotName(source_slot) << " in "
            << super_device;
  return builder;
}

bool DynamicPartitionControlAndroid::StoreMetadata(
    const std::string& super_device,
    MetadataBuilder* builder,
    uint32_t target_slot) {
  auto metadata = builder->Export();
  if (metadata == nullptr) {
    LOG(ERROR) << "Cannot export metadata to slot "
               << BootControlInterface::SlotName(target_slot) << " in "
               << super_device;
    return false;
  }

  if (GetDynamicPartitionsFeatureFlag().IsRetrofit()) {
    if (!FlashPartitionTable(super_device, *metadata)) {
      LOG(ERROR) << "Cannot write metadata to " << super_device;
      return false;
    }
    LOG(INFO) << "Written metadata to " << super_device;
  } else {
    if (!UpdatePartitionTable(super_device, *metadata, target_slot)) {
      LOG(ERROR) << "Cannot write metadata to slot "
                 << BootControlInterface::SlotName(target_slot) << " in "
                 << super_device;
      return false;
    }
    LOG(INFO) << "Copied metadata to slot "
              << BootControlInterface::SlotName(target_slot) << " in "
              << super_device;
  }

  return true;
}

bool DynamicPartitionControlAndroid::GetDeviceDir(std::string* out) {
  // We can't use fs_mgr to look up |partition_name| because fstab
  // doesn't list every slot partition (it uses the slotselect option
  // to mask the suffix).
  //
  // We can however assume that there's an entry for the /misc mount
  // point and use that to get the device file for the misc
  // partition. This helps us locate the disk that |partition_name|
  // resides on. From there we'll assume that a by-name scheme is used
  // so we can just replace the trailing "misc" by the given
  // |partition_name| and suffix corresponding to |slot|, e.g.
  //
  //   /dev/block/platform/soc.0/7824900.sdhci/by-name/misc ->
  //   /dev/block/platform/soc.0/7824900.sdhci/by-name/boot_a
  //
  // If needed, it's possible to relax the by-name assumption in the
  // future by trawling /sys/block looking for the appropriate sibling
  // of misc and then finding an entry in /dev matching the sysfs
  // entry.

  std::string err, misc_device = get_bootloader_message_blk_device(&err);
  if (misc_device.empty()) {
    LOG(ERROR) << "Unable to get misc block device: " << err;
    return false;
  }

  if (!utils::IsSymlink(misc_device.c_str())) {
    LOG(ERROR) << "Device file " << misc_device << " for /misc "
               << "is not a symlink.";
    return false;
  }
  *out = base::FilePath(misc_device).DirName().value();
  return true;
}

bool DynamicPartitionControlAndroid::PreparePartitionsForUpdate(
    uint32_t source_slot,
    uint32_t target_slot,
    const DeltaArchiveManifest& manifest,
    bool update,
    uint64_t* required_size,
    ErrorCode* error) {
  source_slot_ = source_slot;
  target_slot_ = target_slot;
  if (required_size != nullptr) {
    *required_size = 0;
  }

  if (fs_mgr_overlayfs_is_setup()) {
    // Non DAP devices can use overlayfs as well.
    LOG(ERROR)
        << "overlayfs overrides are active and can interfere with our "
           "resources.\n"
        << "run adb enable-verity to deactivate if required and try again.";
    if (error) {
      *error = ErrorCode::kOverlayfsenabledError;
      return false;
    }
  }

  // If metadata is erased but not formatted, it is possible to not mount
  // it in recovery. It is acceptable to skip mounting and choose fallback path
  // (PrepareDynamicPartitionsForUpdate) when sideloading full OTAs.
  TEST_AND_RETURN_FALSE(EnsureMetadataMounted() || IsRecovery());

  if (update) {
    TEST_AND_RETURN_FALSE(EraseSystemOtherAvbFooter(source_slot, target_slot));
  }

  if (!GetDynamicPartitionsFeatureFlag().IsEnabled()) {
    return true;
  }

  if (target_slot == source_slot) {
    LOG(ERROR) << "Cannot call PreparePartitionsForUpdate on current slot.";
    return false;
  }

  if (!SetTargetBuildVars(manifest)) {
    return false;
  }
  for (auto& list : dynamic_partition_list_) {
    list.clear();
  }

  // Although the current build supports dynamic partitions, the given payload
  // doesn't use it for target partitions. This could happen when applying a
  // retrofit update. Skip updating the partition metadata for the target slot.
  if (!is_target_dynamic_) {
    return true;
  }

  if (!update)
    return true;

  bool delete_source = false;

  if (GetVirtualAbFeatureFlag().IsEnabled()) {
    // On Virtual A/B device, either CancelUpdate() or BeginUpdate() must be
    // called before calling UnmapUpdateSnapshot.
    // - If target_supports_snapshot_, PrepareSnapshotPartitionsForUpdate()
    //   calls BeginUpdate() which resets update state
    // - If !target_supports_snapshot_ or PrepareSnapshotPartitionsForUpdate
    //   failed in recovery, explicitly CancelUpdate().
    if (target_supports_snapshot_) {
      if (PrepareSnapshotPartitionsForUpdate(
              source_slot, target_slot, manifest, required_size)) {
        return true;
      }

      // Virtual A/B device doing Virtual A/B update in Android mode must use
      // snapshots.
      if (!IsRecovery()) {
        LOG(ERROR) << "PrepareSnapshotPartitionsForUpdate failed in Android "
                   << "mode";
        return false;
      }

      delete_source = true;
      LOG(INFO) << "PrepareSnapshotPartitionsForUpdate failed in recovery. "
                << "Attempt to overwrite existing partitions if possible";
    } else {
      // Downgrading to an non-Virtual A/B build or is secondary OTA.
      LOG(INFO) << "Using regular A/B on Virtual A/B because package disabled "
                << "snapshots.";
    }

    // In recovery, if /metadata is not mounted, it is likely that metadata
    // partition is erased and not formatted yet. After sideloading, when
    // rebooting into the new version, init will erase metadata partition,
    // hence the failure of CancelUpdate() can be ignored here.
    // However, if metadata is mounted and CancelUpdate fails, sideloading
    // should not proceed because during next boot, snapshots will overlay on
    // the devices incorrectly.
    if (ExpectMetadataMounted()) {
      TEST_AND_RETURN_FALSE(snapshot_->CancelUpdate());
    } else {
      LOG(INFO) << "Skip canceling previous update because metadata is not "
                << "mounted";
    }
  }

  // TODO(xunchang) support partial update on non VAB enabled devices.
  TEST_AND_RETURN_FALSE(PrepareDynamicPartitionsForUpdate(
      source_slot, target_slot, manifest, delete_source));

  if (required_size != nullptr) {
    *required_size = 0;
  }
  return true;
}

bool DynamicPartitionControlAndroid::SetTargetBuildVars(
    const DeltaArchiveManifest& manifest) {
  // Precondition: current build supports dynamic partition.
  CHECK(GetDynamicPartitionsFeatureFlag().IsEnabled());

  bool is_target_dynamic =
      !manifest.dynamic_partition_metadata().groups().empty();
  bool target_supports_snapshot =
      manifest.dynamic_partition_metadata().snapshot_enabled();

  if (manifest.partial_update()) {
    // Partial updates requires DAP. On partial updates that does not involve
    // dynamic partitions, groups() can be empty, so also assume
    // is_target_dynamic in this case. This assumption should be safe because we
    // also check target_supports_snapshot below, which presumably also implies
    // target build supports dynamic partition.
    if (!is_target_dynamic) {
      LOG(INFO) << "Assuming target build supports dynamic partitions for "
                   "partial updates.";
      is_target_dynamic = true;
    }

    // Partial updates requires Virtual A/B. Double check that both current
    // build and target build supports Virtual A/B.
    if (!GetVirtualAbFeatureFlag().IsEnabled()) {
      LOG(ERROR) << "Partial update cannot be applied on a device that does "
                    "not support snapshots.";
      return false;
    }
    if (!target_supports_snapshot) {
      LOG(ERROR) << "Cannot apply partial update to a build that does not "
                    "support snapshots.";
      return false;
    }
  }

  // Store the flags.
  is_target_dynamic_ = is_target_dynamic;
  // If !is_target_dynamic_, leave target_supports_snapshot_ unset because
  // snapshots would not work without dynamic partition.
  if (is_target_dynamic_) {
    target_supports_snapshot_ = target_supports_snapshot;
  }
  return true;
}

namespace {
// Try our best to erase AVB footer.
class AvbFooterEraser {
 public:
  explicit AvbFooterEraser(const std::string& path) : path_(path) {}
  bool Erase() {
    // Try to mark the block device read-only. Ignore any
    // failure since this won't work when passing regular files.
    ignore_result(utils::SetBlockDeviceReadOnly(path_, false /* readonly */));

    fd_.reset(new EintrSafeFileDescriptor());
    int flags = O_WRONLY | O_TRUNC | O_CLOEXEC | O_SYNC;
    TEST_AND_RETURN_FALSE(fd_->Open(path_.c_str(), flags));

    // Need to write end-AVB_FOOTER_SIZE to end.
    static_assert(AVB_FOOTER_SIZE > 0);
    off64_t offset = fd_->Seek(-AVB_FOOTER_SIZE, SEEK_END);
    TEST_AND_RETURN_FALSE_ERRNO(offset >= 0);
    uint64_t write_size = AVB_FOOTER_SIZE;
    LOG(INFO) << "Zeroing " << path_ << " @ [" << offset << ", "
              << (offset + write_size) << "] (" << write_size << " bytes)";
    brillo::Blob zeros(write_size);
    TEST_AND_RETURN_FALSE(utils::WriteAll(fd_, zeros.data(), zeros.size()));
    return true;
  }
  ~AvbFooterEraser() {
    TEST_AND_RETURN(fd_ != nullptr && fd_->IsOpen());
    if (!fd_->Close()) {
      LOG(WARNING) << "Failed to close fd for " << path_;
    }
  }

 private:
  std::string path_;
  FileDescriptorPtr fd_;
};

}  // namespace

std::optional<bool>
DynamicPartitionControlAndroid::IsAvbEnabledOnSystemOther() {
  auto prefix = GetProperty(kPostinstallFstabPrefix, "");
  if (prefix.empty()) {
    LOG(WARNING) << "Cannot get " << kPostinstallFstabPrefix;
    return std::nullopt;
  }
  auto path = base::FilePath(prefix).Append("etc/fstab.postinstall").value();
  return IsAvbEnabledInFstab(path);
}

std::optional<bool> DynamicPartitionControlAndroid::IsAvbEnabledInFstab(
    const std::string& path) {
  Fstab fstab;
  if (!ReadFstabFromFile(path, &fstab)) {
    PLOG(WARNING) << "Cannot read fstab from " << path;
    if (errno == ENOENT) {
      return false;
    }
    return std::nullopt;
  }
  for (const auto& entry : fstab) {
    if (!entry.avb_keys.empty()) {
      return true;
    }
  }
  return false;
}

bool DynamicPartitionControlAndroid::GetSystemOtherPath(
    uint32_t source_slot,
    uint32_t target_slot,
    const std::string& partition_name_suffix,
    std::string* path,
    bool* should_unmap) {
  path->clear();
  *should_unmap = false;

  // Check that AVB is enabled on system_other before erasing.
  auto has_avb = IsAvbEnabledOnSystemOther();
  TEST_AND_RETURN_FALSE(has_avb.has_value());
  if (!has_avb.value()) {
    LOG(INFO) << "AVB is not enabled on system_other. Skip erasing.";
    return true;
  }

  if (!IsRecovery()) {
    // Found unexpected avb_keys for system_other on devices retrofitting
    // dynamic partitions. Previous crash in update_engine may leave logical
    // partitions mapped on physical system_other partition. It is difficult to
    // handle these cases. Just fail.
    if (GetDynamicPartitionsFeatureFlag().IsRetrofit()) {
      LOG(ERROR) << "Cannot erase AVB footer on system_other on devices with "
                 << "retrofit dynamic partitions. They should not have AVB "
                 << "enabled on system_other.";
      return false;
    }
  }

  std::string device_dir_str;
  TEST_AND_RETURN_FALSE(GetDeviceDir(&device_dir_str));
  base::FilePath device_dir(device_dir_str);

  // On devices without dynamic partition, search for static partitions.
  if (!GetDynamicPartitionsFeatureFlag().IsEnabled()) {
    *path = device_dir.Append(partition_name_suffix).value();
    TEST_AND_RETURN_FALSE(DeviceExists(*path));
    return true;
  }

  auto source_super_device =
      device_dir.Append(GetSuperPartitionName(source_slot)).value();

  auto builder = LoadMetadataBuilder(source_super_device, source_slot);
  if (builder == nullptr) {
    if (IsRecovery()) {
      // It might be corrupted for some reason. It should still be able to
      // sideload.
      LOG(WARNING) << "Super partition metadata cannot be read from the source "
                   << "slot, skip erasing.";
      return true;
    } else {
      // Device has booted into Android mode, indicating that the super
      // partition metadata should be there.
      LOG(ERROR) << "Super partition metadata cannot be read from the source "
                 << "slot. This is unexpected on devices with dynamic "
                 << "partitions enabled.";
      return false;
    }
  }
  auto p = builder->FindPartition(partition_name_suffix);
  if (p == nullptr) {
    // If the source slot is flashed without system_other, it does not exist
    // in super partition metadata at source slot. It is safe to skip it.
    LOG(INFO) << "Can't find " << partition_name_suffix
              << " in metadata source slot, skip erasing.";
    return true;
  }
  // System_other created by flashing tools should be erased.
  // If partition is created by update_engine (via NewForUpdate), it is a
  // left-over partition from the previous update and does not contain
  // system_other, hence there is no need to erase.
  // Note the reverse is not necessary true. If the flag is not set, we don't
  // know if the partition is created by update_engine or by flashing tools
  // because older versions of super partition metadata does not contain this
  // flag. It is okay to erase the AVB footer anyways.
  if (p->attributes() & LP_PARTITION_ATTR_UPDATED) {
    LOG(INFO) << partition_name_suffix
              << " does not contain system_other, skip erasing.";
    return true;
  }

  if (p->size() < AVB_FOOTER_SIZE) {
    LOG(INFO) << partition_name_suffix << " has length " << p->size()
              << "( < AVB_FOOTER_SIZE " << AVB_FOOTER_SIZE
              << "), skip erasing.";
    return true;
  }

  // Delete any pre-existing device with name |partition_name_suffix| and
  // also remove it from |mapped_devices_|.
  // In recovery, metadata might not be mounted, and
  // UnmapPartitionOnDeviceMapper might fail. However,
  // it is unusual that system_other has already been mapped. Hence, just skip.
  TEST_AND_RETURN_FALSE(UnmapPartitionOnDeviceMapper(partition_name_suffix));
  // Use CreateLogicalPartition directly to avoid mapping with existing
  // snapshots.
  CreateLogicalPartitionParams params = {
      .block_device = source_super_device,
      .metadata_slot = source_slot,
      .partition_name = partition_name_suffix,
      .force_writable = true,
      .timeout_ms = kMapTimeout,
  };
  TEST_AND_RETURN_FALSE(CreateLogicalPartition(params, path));
  *should_unmap = true;
  return true;
}

bool DynamicPartitionControlAndroid::EraseSystemOtherAvbFooter(
    uint32_t source_slot, uint32_t target_slot) {
  LOG(INFO) << "Erasing AVB footer of system_other partition before update.";

  const std::string target_suffix = SlotSuffixForSlotNumber(target_slot);
  const std::string partition_name_suffix = "system" + target_suffix;

  std::string path;
  bool should_unmap = false;

  TEST_AND_RETURN_FALSE(GetSystemOtherPath(
      source_slot, target_slot, partition_name_suffix, &path, &should_unmap));

  if (path.empty()) {
    return true;
  }

  bool ret = AvbFooterEraser(path).Erase();

  // Delete |partition_name_suffix| from device mapper and from
  // |mapped_devices_| again so that it does not interfere with update process.
  // In recovery, metadata might not be mounted, and
  // UnmapPartitionOnDeviceMapper might fail. However, DestroyLogicalPartition
  // should be called. If DestroyLogicalPartition does fail, it is still okay
  // to skip the error here and let Prepare*() fail later.
  if (should_unmap) {
    TEST_AND_RETURN_FALSE(UnmapPartitionOnDeviceMapper(partition_name_suffix));
  }

  return ret;
}

bool DynamicPartitionControlAndroid::PrepareDynamicPartitionsForUpdate(
    uint32_t source_slot,
    uint32_t target_slot,
    const DeltaArchiveManifest& manifest,
    bool delete_source) {
  const std::string target_suffix = SlotSuffixForSlotNumber(target_slot);

  // Unmap all the target dynamic partitions because they would become
  // inconsistent with the new metadata.
  for (const auto& group : manifest.dynamic_partition_metadata().groups()) {
    for (const auto& partition_name : group.partition_names()) {
      if (!UnmapPartitionOnDeviceMapper(partition_name + target_suffix)) {
        return false;
      }
    }
  }

  std::string device_dir_str;
  TEST_AND_RETURN_FALSE(GetDeviceDir(&device_dir_str));
  base::FilePath device_dir(device_dir_str);
  auto source_device =
      device_dir.Append(GetSuperPartitionName(source_slot)).value();

  auto builder = LoadMetadataBuilder(source_device, source_slot, target_slot);
  if (builder == nullptr) {
    LOG(ERROR) << "No metadata at "
               << BootControlInterface::SlotName(source_slot);
    return false;
  }

  if (delete_source) {
    TEST_AND_RETURN_FALSE(
        DeleteSourcePartitions(builder.get(), source_slot, manifest));
  }

  TEST_AND_RETURN_FALSE(
      UpdatePartitionMetadata(builder.get(), target_slot, manifest));

  auto target_device =
      device_dir.Append(GetSuperPartitionName(target_slot)).value();

  return StoreMetadata(target_device, builder.get(), target_slot);
}

DynamicPartitionControlAndroid::SpaceLimit
DynamicPartitionControlAndroid::GetSpaceLimit(bool use_snapshot) {
  // On device retrofitting dynamic partitions, allocatable_space = "super",
  // where "super" is the sum of all block devices for that slot. Since block
  // devices are dedicated for the corresponding slot, there's no need to halve
  // the allocatable space.
  if (GetDynamicPartitionsFeatureFlag().IsRetrofit())
    return SpaceLimit::ERROR_IF_EXCEEDED_SUPER;

  // On device launching dynamic partitions w/o VAB, regardless of recovery
  // sideload, super partition must be big enough to hold both A and B slots of
  // groups. Hence,
  // allocatable_space = super / 2
  if (!GetVirtualAbFeatureFlag().IsEnabled())
    return SpaceLimit::ERROR_IF_EXCEEDED_HALF_OF_SUPER;

  // Source build supports VAB. Super partition must be big enough to hold
  // one slot of groups (ERROR_IF_EXCEEDED_SUPER). However, there are cases
  // where additional warning messages needs to be written.

  // If using snapshot updates, implying that target build also uses VAB,
  // allocatable_space = super
  if (use_snapshot)
    return SpaceLimit::ERROR_IF_EXCEEDED_SUPER;

  // Source build supports VAB but not using snapshot updates. There are
  // several cases, as listed below.
  // Sideloading: allocatable_space = super.
  if (IsRecovery())
    return SpaceLimit::ERROR_IF_EXCEEDED_SUPER;

  // On launch VAB device, this implies secondary payload.
  // Technically, we don't have to check anything, but sum(groups) < super
  // still applies.
  if (!GetVirtualAbFeatureFlag().IsRetrofit())
    return SpaceLimit::ERROR_IF_EXCEEDED_SUPER;

  // On retrofit VAB device, either of the following:
  // - downgrading: allocatable_space = super / 2
  // - secondary payload: don't check anything
  // These two cases are indistinguishable,
  // hence emit warning if sum(groups) > super / 2
  return SpaceLimit::WARN_IF_EXCEEDED_HALF_OF_SUPER;
}

bool DynamicPartitionControlAndroid::CheckSuperPartitionAllocatableSpace(
    android::fs_mgr::MetadataBuilder* builder,
    const DeltaArchiveManifest& manifest,
    bool use_snapshot) {
  uint64_t sum_groups = 0;
  for (const auto& group : manifest.dynamic_partition_metadata().groups()) {
    sum_groups += group.size();
  }

  uint64_t full_space = builder->AllocatableSpace();
  uint64_t half_space = full_space / 2;
  constexpr const char* fmt =
      "The maximum size of all groups for the target slot (%" PRIu64
      ") has exceeded %sallocatable space for dynamic partitions %" PRIu64 ".";
  switch (GetSpaceLimit(use_snapshot)) {
    case SpaceLimit::ERROR_IF_EXCEEDED_HALF_OF_SUPER: {
      if (sum_groups > half_space) {
        LOG(ERROR) << StringPrintf(fmt, sum_groups, "HALF OF ", half_space);
        return false;
      }
      // If test passes, it implies that the following two conditions also pass.
      break;
    }
    case SpaceLimit::WARN_IF_EXCEEDED_HALF_OF_SUPER: {
      if (sum_groups > half_space) {
        LOG(WARNING) << StringPrintf(fmt, sum_groups, "HALF OF ", half_space)
                     << " This is allowed for downgrade or secondary OTA on "
                        "retrofit VAB device.";
      }
      // still check sum(groups) < super
      [[fallthrough]];
    }
    case SpaceLimit::ERROR_IF_EXCEEDED_SUPER: {
      if (sum_groups > full_space) {
        LOG(ERROR) << base::StringPrintf(fmt, sum_groups, "", full_space);
        return false;
      }
      break;
    }
  }

  return true;
}

bool DynamicPartitionControlAndroid::PrepareSnapshotPartitionsForUpdate(
    uint32_t source_slot,
    uint32_t target_slot,
    const DeltaArchiveManifest& manifest,
    uint64_t* required_size) {
  TEST_AND_RETURN_FALSE(ExpectMetadataMounted());

  std::string device_dir_str;
  TEST_AND_RETURN_FALSE(GetDeviceDir(&device_dir_str));
  base::FilePath device_dir(device_dir_str);
  auto super_device =
      device_dir.Append(GetSuperPartitionName(source_slot)).value();
  auto builder = LoadMetadataBuilder(super_device, source_slot);
  if (builder == nullptr) {
    LOG(ERROR) << "No metadata at "
               << BootControlInterface::SlotName(source_slot);
    return false;
  }

  TEST_AND_RETURN_FALSE(
      CheckSuperPartitionAllocatableSpace(builder.get(), manifest, true));

  if (!snapshot_->BeginUpdate()) {
    LOG(ERROR) << "Cannot begin new update.";
    return false;
  }
  auto ret = snapshot_->CreateUpdateSnapshots(manifest);
  if (!ret) {
    LOG(ERROR) << "Cannot create update snapshots: " << ret.string();
    if (required_size != nullptr &&
        ret.error_code() == Return::ErrorCode::NO_SPACE) {
      *required_size = ret.required_size();
    }
    return false;
  }
  return true;
}

std::string DynamicPartitionControlAndroid::GetSuperPartitionName(
    uint32_t slot) {
  return fs_mgr_get_super_partition_name(slot);
}

bool DynamicPartitionControlAndroid::UpdatePartitionMetadata(
    MetadataBuilder* builder,
    uint32_t target_slot,
    const DeltaArchiveManifest& manifest) {
  // Check preconditions.
  if (GetVirtualAbFeatureFlag().IsEnabled()) {
    CHECK(!target_supports_snapshot_ || IsRecovery())
        << "Must use snapshot on VAB device when target build supports VAB and "
           "not sideloading.";
    LOG_IF(INFO, !target_supports_snapshot_)
        << "Not using snapshot on VAB device because target build does not "
           "support snapshot. Secondary or downgrade OTA?";
    LOG_IF(INFO, IsRecovery())
        << "Not using snapshot on VAB device because sideloading.";
  }

  // If applying downgrade from Virtual A/B to non-Virtual A/B, the left-over
  // COW group needs to be deleted to ensure there are enough space to create
  // target partitions.
  builder->RemoveGroupAndPartitions(android::snapshot::kCowGroupName);

  const std::string target_suffix = SlotSuffixForSlotNumber(target_slot);
  DeleteGroupsWithSuffix(builder, target_suffix);

  TEST_AND_RETURN_FALSE(
      CheckSuperPartitionAllocatableSpace(builder, manifest, false));

  // name of partition(e.g. "system") -> size in bytes
  std::map<std::string, uint64_t> partition_sizes;
  for (const auto& partition : manifest.partitions()) {
    partition_sizes.emplace(partition.partition_name(),
                            partition.new_partition_info().size());
  }

  for (const auto& group : manifest.dynamic_partition_metadata().groups()) {
    auto group_name_suffix = group.name() + target_suffix;
    if (!builder->AddGroup(group_name_suffix, group.size())) {
      LOG(ERROR) << "Cannot add group " << group_name_suffix << " with size "
                 << group.size();
      return false;
    }
    LOG(INFO) << "Added group " << group_name_suffix << " with size "
              << group.size();

    for (const auto& partition_name : group.partition_names()) {
      auto partition_sizes_it = partition_sizes.find(partition_name);
      if (partition_sizes_it == partition_sizes.end()) {
        // TODO(tbao): Support auto-filling partition info for framework-only
        // OTA.
        LOG(ERROR) << "dynamic_partition_metadata contains partition "
                   << partition_name << " but it is not part of the manifest. "
                   << "This is not supported.";
        return false;
      }
      uint64_t partition_size = partition_sizes_it->second;

      auto partition_name_suffix = partition_name + target_suffix;
      Partition* p = builder->AddPartition(
          partition_name_suffix, group_name_suffix, LP_PARTITION_ATTR_READONLY);
      if (!p) {
        LOG(ERROR) << "Cannot add partition " << partition_name_suffix
                   << " to group " << group_name_suffix;
        return false;
      }
      if (!builder->ResizePartition(p, partition_size)) {
        LOG(ERROR) << "Cannot resize partition " << partition_name_suffix
                   << " to size " << partition_size << ". Not enough space?";
        return false;
      }
      if (p->size() < partition_size) {
        LOG(ERROR) << "Partition " << partition_name_suffix
                   << " was expected to have size " << partition_size
                   << ", but instead has size " << p->size();
        return false;
      }
      LOG(INFO) << "Added partition " << partition_name_suffix << " to group "
                << group_name_suffix << " with size " << partition_size;
    }
  }

  return true;
}

bool DynamicPartitionControlAndroid::FinishUpdate(bool powerwash_required) {
  if (ExpectMetadataMounted()) {
    if (snapshot_->GetUpdateState() == UpdateState::Initiated) {
      LOG(INFO) << "Snapshot writes are done.";
      return snapshot_->FinishedSnapshotWrites(powerwash_required);
    }
  } else {
    LOG(INFO) << "Skip FinishedSnapshotWrites() because /metadata is not "
              << "mounted";
  }
  return true;
}

bool DynamicPartitionControlAndroid::GetPartitionDevice(
    const std::string& partition_name,
    uint32_t slot,
    uint32_t current_slot,
    bool not_in_payload,
    std::string* device,
    bool* is_dynamic) {
  auto partition_dev =
      GetPartitionDevice(partition_name, slot, current_slot, not_in_payload);
  if (!partition_dev.has_value()) {
    return false;
  }
  if (device) {
    *device = std::move(partition_dev->rw_device_path);
  }
  if (is_dynamic) {
    *is_dynamic = partition_dev->is_dynamic;
  }
  return true;
}

bool DynamicPartitionControlAndroid::GetPartitionDevice(
    const std::string& partition_name,
    uint32_t slot,
    uint32_t current_slot,
    std::string* device) {
  return GetPartitionDevice(
      partition_name, slot, current_slot, false, device, nullptr);
}

static std::string GetStaticDevicePath(
    const base::FilePath& device_dir,
    const std::string& partition_name_suffixed) {
  base::FilePath path = device_dir.Append(partition_name_suffixed);
  return path.value();
}

std::optional<PartitionDevice>
DynamicPartitionControlAndroid::GetPartitionDevice(
    const std::string& partition_name,
    uint32_t slot,
    uint32_t current_slot,
    bool not_in_payload) {
  std::string device_dir_str;
  if (!GetDeviceDir(&device_dir_str)) {
    LOG(ERROR) << "Failed to GetDeviceDir()";
    return {};
  }
  const base::FilePath device_dir(device_dir_str);
  // When VABC is enabled, we can't get device path for dynamic partitions in
  // target slot.
  const auto& partition_name_suffix =
      partition_name + SlotSuffixForSlotNumber(slot);
  if (UpdateUsesSnapshotCompression() && slot != current_slot &&
      IsDynamicPartition(partition_name, slot)) {
    return {
        {.readonly_device_path = base::FilePath{std::string{VABC_DEVICE_DIR}}
                                     .Append(partition_name_suffix)
                                     .value(),
         .is_dynamic = true}};
  }

  // When looking up target partition devices, treat them as static if the
  // current payload doesn't encode them as dynamic partitions. This may happen
  // when applying a retrofit update on top of a dynamic-partitions-enabled
  // build.
  std::string device;
  if (GetDynamicPartitionsFeatureFlag().IsEnabled() &&
      (slot == current_slot || is_target_dynamic_)) {
    switch (GetDynamicPartitionDevice(device_dir,
                                      partition_name_suffix,
                                      slot,
                                      current_slot,
                                      not_in_payload,
                                      &device)) {
      case DynamicPartitionDeviceStatus::SUCCESS:
        return {{.rw_device_path = device,
                 .readonly_device_path = device,
                 .is_dynamic = true}};

      case DynamicPartitionDeviceStatus::TRY_STATIC:
        break;
      case DynamicPartitionDeviceStatus::ERROR:  // fallthrough
      default:
        return {};
    }
  }
  // Try static partitions.
  auto static_path = GetStaticDevicePath(device_dir, partition_name_suffix);
  if (!DeviceExists(static_path)) {
    LOG(ERROR) << "Device file " << static_path << " does not exist.";
    return {};
  }

  return {{.rw_device_path = static_path,
           .readonly_device_path = static_path,
           .is_dynamic = false}};
}

bool DynamicPartitionControlAndroid::IsSuperBlockDevice(
    const base::FilePath& device_dir,
    uint32_t current_slot,
    const std::string& partition_name_suffix) {
  std::string source_device =
      device_dir.Append(GetSuperPartitionName(current_slot)).value();
  auto source_metadata = LoadMetadataBuilder(source_device, current_slot);
  return source_metadata->HasBlockDevice(partition_name_suffix);
}

DynamicPartitionControlAndroid::DynamicPartitionDeviceStatus
DynamicPartitionControlAndroid::GetDynamicPartitionDevice(
    const base::FilePath& device_dir,
    const std::string& partition_name_suffix,
    uint32_t slot,
    uint32_t current_slot,
    bool not_in_payload,
    std::string* device) {
  std::string super_device =
      device_dir.Append(GetSuperPartitionName(slot)).value();

  auto builder = LoadMetadataBuilder(super_device, slot);
  if (builder == nullptr) {
    LOG(ERROR) << "No metadata in slot "
               << BootControlInterface::SlotName(slot);
    return DynamicPartitionDeviceStatus::ERROR;
  }
  if (builder->FindPartition(partition_name_suffix) == nullptr) {
    LOG(INFO) << partition_name_suffix
              << " is not in super partition metadata.";

    if (IsSuperBlockDevice(device_dir, current_slot, partition_name_suffix)) {
      LOG(ERROR) << "The static partition " << partition_name_suffix
                 << " is a block device for current metadata."
                 << "It cannot be used as a logical partition.";
      return DynamicPartitionDeviceStatus::ERROR;
    }

    return DynamicPartitionDeviceStatus::TRY_STATIC;
  }

  if (slot == current_slot) {
    if (GetState(partition_name_suffix) != DmDeviceState::ACTIVE) {
      LOG(WARNING) << partition_name_suffix << " is at current slot but it is "
                   << "not mapped. Now try to map it.";
    } else {
      if (GetDmDevicePathByName(partition_name_suffix, device)) {
        LOG(INFO) << partition_name_suffix
                  << " is mapped on device mapper: " << *device;
        return DynamicPartitionDeviceStatus::SUCCESS;
      }
      LOG(ERROR) << partition_name_suffix << "is mapped but path is unknown.";
      return DynamicPartitionDeviceStatus::ERROR;
    }
  }

  bool force_writable = (slot != current_slot) && !not_in_payload;
  if (MapPartitionOnDeviceMapper(
          super_device, partition_name_suffix, slot, force_writable, device)) {
    return DynamicPartitionDeviceStatus::SUCCESS;
  }
  return DynamicPartitionDeviceStatus::ERROR;
}

void DynamicPartitionControlAndroid::set_fake_mapped_devices(
    const std::set<std::string>& fake) {
  mapped_devices_ = fake;
}

bool DynamicPartitionControlAndroid::IsRecovery() {
  return constants::kIsRecovery;
}

bool DynamicPartitionControlAndroid::DeleteSourcePartitions(
    MetadataBuilder* builder,
    uint32_t source_slot,
    const DeltaArchiveManifest& manifest) {
  TEST_AND_RETURN_FALSE(IsRecovery());

  LOG(INFO) << "Will overwrite existing partitions. Slot "
            << BootControlInterface::SlotName(source_slot)
            << " may be unbootable until update finishes!";
  const std::string source_suffix = SlotSuffixForSlotNumber(source_slot);
  DeleteGroupsWithSuffix(builder, source_suffix);

  return true;
}

std::unique_ptr<AbstractAction>
DynamicPartitionControlAndroid::GetCleanupPreviousUpdateAction(
    BootControlInterface* boot_control,
    PrefsInterface* prefs,
    CleanupPreviousUpdateActionDelegateInterface* delegate) {
  if (!GetVirtualAbFeatureFlag().IsEnabled()) {
    return std::make_unique<NoOpAction>();
  }
  return std::make_unique<CleanupPreviousUpdateAction>(
      prefs, boot_control, snapshot_.get(), delegate);
}

bool DynamicPartitionControlAndroid::ResetUpdate(PrefsInterface* prefs) {
  if (!GetVirtualAbFeatureFlag().IsEnabled()) {
    return true;
  }
  for (auto& list : dynamic_partition_list_) {
    list.clear();
  }

  LOG(INFO) << __func__ << " resetting update state and deleting snapshots.";
  TEST_AND_RETURN_FALSE(prefs != nullptr);

  // If the device has already booted into the target slot,
  // ResetUpdateProgress may pass but CancelUpdate fails.
  // This is expected. A scheduled CleanupPreviousUpdateAction should free
  // space when it is done.
  TEST_AND_RETURN_FALSE(DeltaPerformer::ResetUpdateProgress(
      prefs, false /* quick */, false /* skip dynamic partitions metadata */));

  if (ExpectMetadataMounted()) {
    TEST_AND_RETURN_FALSE(snapshot_->CancelUpdate());
  } else {
    LOG(INFO) << "Skip cancelling update in ResetUpdate because /metadata is "
              << "not mounted";
  }

  return true;
}

bool DynamicPartitionControlAndroid::ListDynamicPartitionsForSlot(
    uint32_t slot,
    uint32_t current_slot,
    std::vector<std::string>* partitions) {
  CHECK(slot == source_slot_ || target_slot_ != UINT32_MAX)
      << " source slot: " << source_slot_ << " target slot: " << target_slot_
      << " slot: " << slot
      << " attempting to query dynamic partition metadata for target slot "
         "before PreparePartitionForUpdate() is called. The "
         "metadata in target slot isn't valid until "
         "PreparePartitionForUpdate() is called, contining execution would "
         "likely cause problems.";
  bool slot_enables_dynamic_partitions =
      GetDynamicPartitionsFeatureFlag().IsEnabled();
  // Check if the target slot has dynamic partitions, this may happen when
  // applying a retrofit package.
  if (slot != current_slot) {
    slot_enables_dynamic_partitions =
        slot_enables_dynamic_partitions && is_target_dynamic_;
  }

  if (!slot_enables_dynamic_partitions) {
    LOG(INFO) << "Dynamic partition is not enabled for slot " << slot;
    return true;
  }

  std::string device_dir_str;
  TEST_AND_RETURN_FALSE(GetDeviceDir(&device_dir_str));
  base::FilePath device_dir(device_dir_str);
  auto super_device = device_dir.Append(GetSuperPartitionName(slot)).value();
  auto builder = LoadMetadataBuilder(super_device, slot);
  TEST_AND_RETURN_FALSE(builder != nullptr);

  std::vector<std::string> result;
  auto suffix = SlotSuffixForSlotNumber(slot);
  for (const auto& group : builder->ListGroups()) {
    for (const auto& partition : builder->ListPartitionsInGroup(group)) {
      std::string_view partition_name = partition->name();
      if (!android::base::ConsumeSuffix(&partition_name, suffix)) {
        continue;
      }
      result.emplace_back(partition_name);
    }
  }
  *partitions = std::move(result);
  return true;
}

bool DynamicPartitionControlAndroid::VerifyExtentsForUntouchedPartitions(
    uint32_t source_slot,
    uint32_t target_slot,
    const std::vector<std::string>& partitions) {
  std::string device_dir_str;
  TEST_AND_RETURN_FALSE(GetDeviceDir(&device_dir_str));
  base::FilePath device_dir(device_dir_str);

  auto source_super_device =
      device_dir.Append(GetSuperPartitionName(source_slot)).value();
  auto source_builder = LoadMetadataBuilder(source_super_device, source_slot);
  TEST_AND_RETURN_FALSE(source_builder != nullptr);

  auto target_super_device =
      device_dir.Append(GetSuperPartitionName(target_slot)).value();
  auto target_builder = LoadMetadataBuilder(target_super_device, target_slot);
  TEST_AND_RETURN_FALSE(target_builder != nullptr);

  return MetadataBuilder::VerifyExtentsAgainstSourceMetadata(
      *source_builder, source_slot, *target_builder, target_slot, partitions);
}

bool DynamicPartitionControlAndroid::ExpectMetadataMounted() {
  // No need to mount metadata for non-Virtual A/B devices.
  if (!GetVirtualAbFeatureFlag().IsEnabled()) {
    return false;
  }
  // Intentionally not checking |metadata_device_| in Android mode.
  // /metadata should always be mounted in Android mode. If it isn't, let caller
  // fails when calling into SnapshotManager.
  if (!IsRecovery()) {
    return true;
  }
  // In recovery mode, explicitly check |metadata_device_|.
  return metadata_device_ != nullptr;
}

bool DynamicPartitionControlAndroid::EnsureMetadataMounted() {
  // No need to mount metadata for non-Virtual A/B devices.
  if (!GetVirtualAbFeatureFlag().IsEnabled()) {
    return true;
  }

  if (metadata_device_ == nullptr) {
    metadata_device_ = snapshot_->EnsureMetadataMounted();
  }
  return metadata_device_ != nullptr;
}

std::unique_ptr<android::snapshot::ICowWriter>
DynamicPartitionControlAndroid::OpenCowWriter(
    const std::string& partition_name,
    const std::optional<std::string>& source_path,
    std::optional<uint64_t> label) {
  auto suffix = SlotSuffixForSlotNumber(target_slot_);

  auto super_device = GetSuperDevice();
  if (!super_device.has_value()) {
    return nullptr;
  }
  CreateLogicalPartitionParams params = {
      .block_device = super_device->value(),
      .metadata_slot = target_slot_,
      .partition_name = partition_name + suffix,
      .force_writable = true,
      .timeout_ms = kMapSnapshotTimeout};
  // TODO(zhangkelvin) Open an APPEND mode CowWriter once there's an API to do
  // it.
  return snapshot_->OpenSnapshotWriter(params, label);
}  // namespace chromeos_update_engine

std::unique_ptr<FileDescriptor> DynamicPartitionControlAndroid::OpenCowFd(
    const std::string& unsuffixed_partition_name,
    const std::optional<std::string>& source_path,
    bool is_append) {
  auto cow_writer = OpenCowWriter(
      unsuffixed_partition_name, source_path, {kEndOfInstallLabel});
  if (cow_writer == nullptr) {
    LOG(ERROR) << "OpenCowWriter failed";
    return nullptr;
  }
  auto fd = cow_writer->OpenFileDescriptor(source_path);
  if (fd == nullptr) {
    LOG(ERROR) << "ICowReader::OpenFileDescriptor failed";
    return nullptr;
  }
  return std::make_unique<CowWriterFileDescriptor>(
      std::move(cow_writer), std::move(fd), source_path);
}

std::optional<base::FilePath> DynamicPartitionControlAndroid::GetSuperDevice() {
  std::string device_dir_str;
  if (!GetDeviceDir(&device_dir_str)) {
    LOG(ERROR) << "Failed to get device dir!";
    return {};
  }
  base::FilePath device_dir(device_dir_str);
  auto super_device = device_dir.Append(GetSuperPartitionName(target_slot_));
  return super_device;
}

bool DynamicPartitionControlAndroid::MapAllPartitions() {
  return snapshot_->MapAllSnapshots(kMapSnapshotTimeout);
}

bool DynamicPartitionControlAndroid::IsDynamicPartition(
    const std::string& partition_name, uint32_t slot) {
  if (slot >= dynamic_partition_list_.size()) {
    LOG(ERROR) << "Seeing unexpected slot # " << slot << " currently assuming "
               << dynamic_partition_list_.size() << " slots";
    return false;
  }
  auto& dynamic_partition_list = dynamic_partition_list_[slot];
  if (dynamic_partition_list.empty() &&
      GetDynamicPartitionsFeatureFlag().IsEnabled()) {
    // Use the DAP config of the target slot.
    CHECK(ListDynamicPartitionsForSlot(
        slot, source_slot_, &dynamic_partition_list));
  }
  return std::find(dynamic_partition_list.begin(),
                   dynamic_partition_list.end(),
                   partition_name) != dynamic_partition_list.end();
}

bool DynamicPartitionControlAndroid::UpdateUsesSnapshotCompression() {
  return GetVirtualAbFeatureFlag().IsEnabled() &&
         snapshot_->UpdateUsesCompression();
}

FeatureFlag
DynamicPartitionControlAndroid::GetVirtualAbUserspaceSnapshotsFeatureFlag() {
  return virtual_ab_userspace_snapshots_;
}

}  // namespace chromeos_update_engine
