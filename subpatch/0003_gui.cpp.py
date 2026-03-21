from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Fix power+volume key (gui.cpp)"
        self.target_file = "bootable/recovery/gui/gui.cpp"

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
			} else {
				blankTimer.toggleBlank();
			}
		} else {
			if (ev.code == KEY_POWER && key_status != KS_KEY_REPEAT) {
				LOGEVENT("POWER Key Released\n");
				blankTimer.toggleBlank();
			}
		}
""",
                # Блок 1: Модифицированный код (результат)
                r"""
			} else {
				blankTimer.toggleBlank();
			}
		} else {
			if (ev.code == KEY_POWER && key_status != KS_KEY_REPEAT
			    && !kb->IsKeyDown(KEY_VOLUMEUP) && !kb->IsKeyDown(KEY_VOLUMEDOWN)) {
				LOGEVENT("POWER Key Released\n");
				blankTimer.toggleBlank();
			}
		}
"""
            )
        ]

