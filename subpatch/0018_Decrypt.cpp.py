from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Custom DE modifications (Decrypt.cpp)"
        self.target_file = "system/vold/Decrypt.cpp" # bootable/recovery/gui/gui.cpp

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
extern "C" bool Decrypt_DE() {
	printf("Attempting to initialize DE keys\n");
	if (!fscrypt_initialize_systemwide_keys()) { // this deals with the overarching device encryption
		printf("fscrypt_initialize_systemwide_keys returned fail\n");
		return false;
	}
	if (!fscrypt_init_user0()) {
		printf("fscrypt_init_user0 returned fail\n");
		return false;
	}
	return true;
}

// Crappy functions for debugging, please ignore unless you need to debug
// void output_hex(const std::string& in) {
// 	const char *buf = in.data();
// 	char hex[in.size() * 2 + 1];
// 	unsigned int index;
// 	for (index = 0; index < in.size(); index++)
// 		sprintf(&hex[2 * index], "%02X", buf[index]);
// }
""",
                # Блок 1: Модифицированный код (результат)
                r"""
extern "C" bool Decrypt_DE() {
	printf("Attempting to initialize DE keys\n");
	if (!fscrypt_initialize_systemwide_keys()) { // this deals with the overarching device encryption
		printf("fscrypt_initialize_systemwide_keys returned fail\n");
		return false;
	}
	if (!fscrypt_init_user0()) {
		printf("fscrypt_init_user0 returned fail\n");
		return false;
	}
	return true;
}

extern "C" bool Reset_FBE_Caches() {
	return fscrypt_reset_key_caches();
}

// Crappy functions for debugging, please ignore unless you need to debug
// void output_hex(const std::string& in) {
// 	const char *buf = in.data();
// 	char hex[in.size() * 2 + 1];
// 	unsigned int index;
// 	for (index = 0; index < in.size(); index++)
// 		sprintf(&hex[2 * index], "%02X", buf[index]);
// }
"""
            )
        ]

