from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Torch custom logic (action.cpp)"
        self.target_file = "bootable/recovery/gui/action.cpp"

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
		DataManager::GetValue("of_fl_path_1", path_one);
		DataManager::GetValue("of_fl_path_2", path_two);
		DataManager::GetValue("of_flash_on", fl_used);
		
		if (path_one.empty() && path_two.empty()) {
			// maintainer not set flash paths
			if (TWFunc::Path_Exists("/sys/class/leds/flashlight/brightness")) {
				// use flashlight for old devices
				path_one = "/sys/class/leds/flashlight";
			} else {
				// use flashlight for new devices
				path_one = "/sys/class/leds/led:torch_0";
				path_two = "/sys/class/leds/led:switch_0";
			}
		} else if (path_one.empty() && !path_two.empty()) {
			path_one = path_two;
			path_two = "";
		}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
		DataManager::GetValue("of_fl_path_1", path_one);
		DataManager::GetValue("of_fl_path_2", path_two);
		DataManager::GetValue("of_flash_on", fl_used);

		// Support command-based torch control: if path starts with "cmd:" treat rest as script
		if (!path_one.empty() && path_one.substr(0, 4) == "cmd:") {
			std::string cmd = path_one.substr(4);
			if (fl_used != "1") {
				TWFunc::Exec_Cmd(cmd + " on");
				DataManager::SetValue("of_flash_on", "1");
			} else {
				TWFunc::Exec_Cmd(cmd + " off");
				DataManager::SetValue("of_flash_on", "0");
			}
			return 0;
		}

		if (path_one.empty() && path_two.empty()) {
			// maintainer not set flash paths
			if (TWFunc::Path_Exists("/sys/class/leds/flashlight/brightness")) {
				// use flashlight for old devices
				path_one = "/sys/class/leds/flashlight";
			} else {
				// use flashlight for new devices
				path_one = "/sys/class/leds/led:torch_0";
				path_two = "/sys/class/leds/led:switch_0";
			}
		} else if (path_one.empty() && !path_two.empty()) {
			path_one = path_two;
			path_two = "";
		}
"""
            )
        ]
