from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Disable wipe after flash (install.cpp)"
        self.target_file = "bootable/recovery/install/install.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
static bool PerformPowerwashIfRequired(ZipArchiveHandle zip) {
  const auto payload_properties = ExtractPayloadProperties(zip);
  if (payload_properties.find("POWERWASH=1") != std::string::npos) {
    LOG(INFO) << "Payload properties has POWERWASH=1, wiping userdata...";
    return WipeData(nullptr);
  }
  return true;
}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
static bool PerformPowerwashIfRequired(ZipArchiveHandle zip) {
  // OrangeFox: forced powerwash disabled — never wipe data from OTA
  const auto payload_properties = ExtractPayloadProperties(zip);
  if (payload_properties.find("POWERWASH=1") != std::string::npos) {
    LOG(WARNING) << "Payload properties has POWERWASH=1, but powerwash is disabled. Skipping wipe.";
  }
  return true;
}
"""
            )
        ]

