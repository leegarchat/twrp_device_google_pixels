from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom hardware modifications (hardware_android.cc)"
        self.target_file = "system/update_engine/aosp/hardware_android.cc" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
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
""",
                # Блок 1: Модифицированный код (результат)
                r"""
bool HardwareAndroid::SchedulePowerwash(bool /* save_rollback_data */) {
  // OrangeFox: powerwash disabled — never write --wipe_data to BCB
  LOG(WARNING) << "SchedulePowerwash called but disabled. Not writing to BCB.";
  return true;
}
"""
            )
        ]

