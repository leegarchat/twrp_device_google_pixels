from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom OTA modifications (delta_performer.cc)"
        self.target_file = "system/update_engine/payload_consumer/delta_performer.cc" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
bool DeltaPerformer::CheckSPLDowngrade() {
  if (!manifest_.has_security_patch_level()) {
    return true;
  }
  if (manifest_.security_patch_level().empty()) {
    return true;
  }
  const auto new_spl = manifest_.security_patch_level();
  const auto current_spl =
      android::base::GetProperty("ro.build.version.security_patch", "");
  if (current_spl.empty()) {
    LOG(WARNING) << "Failed to get ro.build.version.security_patch, unable to "
                    "determine if this OTA is a SPL downgrade. Assuming this "
                    "OTA is not SPL downgrade.";
    return true;
  }
  if (new_spl < current_spl) {
    const auto avb_state =
        android::base::GetProperty("ro.boot.verifiedbootstate", "green");
    if (android::base::EqualsIgnoreCase(avb_state, "green")) {
      LOG(ERROR) << "Target build SPL " << new_spl
                 << " is older than current build's SPL " << current_spl
                 << ", this OTA is an SPL downgrade. Your device's "
                    "ro.boot.verifiedbootstate="
                 << avb_state
                 << ", it probably has a locked bootlaoder. Since a locked "
                    "bootloader will reject SPL downgrade no matter what, we "
                    "will reject this OTA.";
      return false;
    }
    install_plan_->powerwash_required = true;
    LOG(WARNING)
        << "Target build SPL " << new_spl
        << " is older than current build's SPL " << current_spl
        << ", this OTA is an SPL downgrade. Data wipe will be required";
  }
  return true;
}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
bool DeltaPerformer::CheckSPLDowngrade() {
  // OrangeFox: SPL downgrade check disabled — allow any OTA without forced wipe
  LOG(INFO) << "CheckSPLDowngrade: check disabled, permitting OTA";
  return true;
}
"""
            ),
            (
                # Блок 1: Оригинальный код для поиска
                r"""
    if (downgrade_detected) {
      return ErrorCode::kPayloadTimestampError;
    }
    return ErrorCode::kSuccess;
  }

  // For non-partial updates, check max_timestamp first.
  if (hardware_->IsOfficialBuild() && manifest_.max_timestamp() < hardware_->GetBuildTimestamp()) {
    LOG(ERROR) << "The current OS build timestamp ("
               << hardware_->GetBuildTimestamp()
               << ") is newer than the maximum timestamp in the manifest ("
               << manifest_.max_timestamp() << ")";
    return ErrorCode::kPayloadTimestampError;
  }
  // Otherwise... partitions can have empty timestamps.
  for (const auto& partition : partitions) {
    auto error_code = timestamp_valid(
        partition, true /* allow_empty_version */, &downgrade_detected);
    if (error_code != ErrorCode::kSuccess &&
        error_code != ErrorCode::kPayloadTimestampError) {
      return error_code;
    }
  }
""",
                # Блок 1: Модифицированный код (результат)
                r"""
    if (downgrade_detected) {
      return ErrorCode::kPayloadTimestampError;
    }
    return ErrorCode::kSuccess;
  }

  // For non-partial updates, check max_timestamp first.
  // OrangeFox: timestamp downgrade check disabled
  if (hardware_->IsOfficialBuild() && manifest_.max_timestamp() < hardware_->GetBuildTimestamp()) {
    LOG(WARNING) << "Build timestamp downgrade detected ("
                 << hardware_->GetBuildTimestamp() << " > "
                 << manifest_.max_timestamp()
                 << ") but check is disabled, permitting OTA";
  }
  // Otherwise... partitions can have empty timestamps.
  for (const auto& partition : partitions) {
    auto error_code = timestamp_valid(
        partition, true /* allow_empty_version */, &downgrade_detected);
    if (error_code != ErrorCode::kSuccess &&
        error_code != ErrorCode::kPayloadTimestampError) {
      return error_code;
    }
  }
"""
            )
        ]

