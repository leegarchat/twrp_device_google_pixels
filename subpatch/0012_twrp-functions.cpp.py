from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Unmount /data (twrp-functions.cpp)"
        self.target_file = "bootable/recovery/twrp-functions.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
	if (PartitionManager.Is_Mounted_By_Path("/sdcard")) {
		string mycmd = "/system/bin/umount /sdcard";
		Exec_Cmd(mycmd);
		usleep(262144);
	}
	#endif

	// unmount /data
	TWPartition *dataPart = PartitionManager.Find_Partition_By_Path("/data");
	if (dataPart) {
		if (dataPart->Is_Mounted()) {
			if (!dataPart->UnMount(false)) {
				usleep(16384);
				dataPart->UnMount(false, MNT_FORCE | MNT_DETACH);
			}
			usleep(262144);
		}
	}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
	if (PartitionManager.Is_Mounted_By_Path("/sdcard")) {
		string mycmd = "/system/bin/umount /sdcard";
		Exec_Cmd(mycmd);
		usleep(262144);
	}
	#endif

	// Force f2fs checkpoint and sync BEFORE any unmount attempts.
	// With lazy unmount (MNT_DETACH), f2fs may not commit a checkpoint,
	// causing newly written files to be lost after reboot.
	sync();

	// unmount /data — must remove all bind mounts first, otherwise
	// umount("/data") returns EBUSY and falls back to lazy detach,
	// which doesn't guarantee f2fs checkpoint commit.
	TWPartition *dataPart = PartitionManager.Find_Partition_By_Path("/data");
	if (dataPart) {
		if (dataPart->Is_Mounted()) {
			// Stop MTP first — it holds fds on /data/media preventing clean unmount
			PartitionManager.Disable_MTP();

			// Parse /proc/mounts to find ALL bind/overlay mounts that sit on top
			// of /data (e.g. /sdcard, /data/user/0, /data/media/0 — sometimes
			// duplicated). Unmount them in reverse order so /data itself can be
			// cleanly unmounted, which guarantees an f2fs checkpoint write.
			std::vector<std::string> data_submounts;
			std::ifstream mounts("/proc/mounts");
			if (mounts.is_open()) {
				std::string line;
				while (std::getline(mounts, line)) {
					std::istringstream iss(line);
					std::string dev, mp;
					iss >> dev >> mp;
					// Collect everything mounted on the same device that isn't /data itself
					if ((mp.rfind("/data/", 0) == 0 || mp == "/sdcard") && mp != "/data") {
						data_submounts.push_back(mp);
					}
				}
				mounts.close();
			}
			// Reverse order: deepest paths first
			std::sort(data_submounts.begin(), data_submounts.end(), std::greater<std::string>());
			for (const auto& mp : data_submounts) {
				LOGINFO("Pre-reboot: unmounting sub-mount %s\n", mp.c_str());
				if (umount2(mp.c_str(), 0) != 0)
					umount2(mp.c_str(), MNT_DETACH);
			}

			if (!dataPart->UnMount(false)) {
				usleep(16384);
				dataPart->UnMount(false, MNT_FORCE | MNT_DETACH);
			}
			usleep(262144);
		}
	}
"""
            )
        ]

