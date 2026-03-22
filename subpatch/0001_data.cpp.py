from patch import BaseSubPatch, Colors

class SubPatch(BaseSubPatch):
    def __init__(self, manager):
        super().__init__(manager)
        self.name = "Dynamic Screen Height (data.cpp)"
        self.target_file = "bootable/recovery/data.cpp"

        # Список изменений: [(Оригинал, Модификация), ...]
        self.CHANGES = [
            (
                # Блок 1: Оригинальный код для поиска
                r"""
  int of_status_placement = (atoi(OF_STATUS_H) / 2) - 28;
  int of_center_y = atoi(OF_SCREEN_H) / 2;
  
  mConst.SetValue(OF_STATUS_PLACEMENT_S, of_status_placement);
  mConst.SetValue(OF_CENTER_Y_S, of_center_y);
  
  mConst.SetValue(OF_SCREEN_H_S, OF_SCREEN_H);
  mData.SetValue(OF_SCREEN_NAV_H_S, OF_SCREEN_H); // mData for nide navbar function
  
  mConst.SetValue(OF_STATUS_H_S, OF_STATUS_H);
  mConst.SetValue(OF_HIDE_NOTCH_S, OF_HIDE_NOTCH);
  mConst.SetValue(OF_STATUS_INDENT_LEFT_S, OF_STATUS_INDENT_LEFT);
  mConst.SetValue(OF_STATUS_INDENT_RIGHT_S, OF_STATUS_INDENT_RIGHT);
  mConst.SetValue(OF_CLOCK_POS_S, OF_CLOCK_POS);
  mConst.SetValue(OF_ALLOW_DISABLE_NAVBAR_S, OF_ALLOW_DISABLE_NAVBAR);
  mConst.SetValue(OF_FLASHLIGHT_ENABLE_STR, OF_FLASHLIGHT_ENABLE);
""",
                # Блок 1: Модифицированный код (результат)
                r"""
  int of_status_placement = (atoi(OF_STATUS_H) / 2) - 28;

  // Dynamic screen height override: if the DOF_SCREEN_H property is set (by runatboot.sh),
  // use it instead of the compile-time OF_SCREEN_H default. This allows devices with
  // different screen heights (e.g. husky=2244) to share a single recovery image.
  char dof_screen_h[PROPERTY_VALUE_MAX] = {0};
  property_get("DOF_SCREEN_H", dof_screen_h, "");
  const char* effective_screen_h = (dof_screen_h[0] != '\0') ? dof_screen_h : OF_SCREEN_H;

  int of_center_y = atoi(effective_screen_h) / 2;
  
  mConst.SetValue(OF_STATUS_PLACEMENT_S, of_status_placement);
  mConst.SetValue(OF_CENTER_Y_S, of_center_y);
  
  mConst.SetValue(OF_SCREEN_H_S, effective_screen_h);
  mData.SetValue(OF_SCREEN_NAV_H_S, effective_screen_h); // mData for nide navbar function
  
  mConst.SetValue(OF_STATUS_H_S, OF_STATUS_H);
  mConst.SetValue(OF_HIDE_NOTCH_S, OF_HIDE_NOTCH);
  mConst.SetValue(OF_STATUS_INDENT_LEFT_S, OF_STATUS_INDENT_LEFT);
  mConst.SetValue(OF_STATUS_INDENT_RIGHT_S, OF_STATUS_INDENT_RIGHT);
  mConst.SetValue(OF_CLOCK_POS_S, OF_CLOCK_POS);
  mConst.SetValue(OF_ALLOW_DISABLE_NAVBAR_S, OF_ALLOW_DISABLE_NAVBAR);
  mConst.SetValue(OF_FLASHLIGHT_ENABLE_STR, OF_FLASHLIGHT_ENABLE);
"""
            )
        ]
