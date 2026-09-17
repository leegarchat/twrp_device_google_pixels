/*
	Copyright 2013 bigbiff/Dees_Troy TeamWin
	This file is part of TWRP/TeamWin Recovery Project.

	Copyright (C) 2018-2025 OrangeFox Recovery Project
	This file is part of the OrangeFox Recovery Project.

	TWRP is free software: you can redistribute it and/or modify
	it under the terms of the GNU General Public License as published by
	the Free Software Foundation, either version 3 of the License, or
	(at your option) any later version.

	TWRP is distributed in the hope that it will be useful,
	but WITHOUT ANY WARRANTY; without even the implied warranty of
	MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
	GNU General Public License for more details.

	You should have received a copy of the GNU General Public License
	along with TWRP.  If not, see <http://www.gnu.org/licenses/>.
*/

#include <string.h>

extern "C" {
#include "../twcommon.h"
}
#include "minuitwrp/minui.h"

#include "rapidxml.hpp"
#include "objects.hpp"
#include "../data.hpp"
#include "../partitions.hpp"
#include "pages.hpp"
#include "../twrp-functions.hpp"

extern std::vector<language_struct> Language_List;

GUIListBox::GUIListBox(xml_node<>* node) : GUIScrollList(node)
{
	xml_attribute<>* attr;
	xml_node<>* child;
	mIconSelected = mIconUnselected = NULL;
	mUpdate = 0;
	requireReload = isCheckList = isTextParsed = false;
	
	// Get the icons, if any
	child = FindNode(node, "icon");
	if (child) {
		mIconSelected = LoadAttrImage(child, "selected");
		mIconUnselected = LoadAttrImage(child, "unselected");
	}
	int iconWidth = 0, iconHeight = 0;
	
	// [f/d] Get size for icons
	child = FindNode(node, "iconsize");
	if (child) {
		iconWidth = LoadAttrIntScaleX(child, "w", iconWidth);
		iconHeight = LoadAttrIntScaleY(child, "h", iconHeight);
	} else {
		if (mIconSelected && mIconSelected->GetResource() && mIconUnselected && mIconUnselected->GetResource()) {
			iconWidth = std::max(mIconSelected->GetWidth(), mIconUnselected->GetWidth());
			iconHeight = std::max(mIconSelected->GetHeight(), mIconUnselected->GetHeight());
		} else if (mIconSelected && mIconSelected->GetResource()) {
			iconWidth = mIconSelected->GetWidth();
			iconHeight = mIconSelected->GetHeight();
		} else if (mIconUnselected && mIconUnselected->GetResource()) {
			iconWidth = mIconUnselected->GetWidth();
			iconHeight = mIconUnselected->GetHeight();
		}
	}

	// [Yacha] Get margin for list items
	child = FindNode(node, "margin");
	if (child) {
		this->mItemPaddingTop = LoadAttrIntScaleY(child, "top", 0);
		this->mItemPaddingBottom = LoadAttrIntScaleY(child, "bottom", 0);
		// If only one value provided, use it for both top and bottom
		if (LoadAttrIntScaleY(child, "size", -1) != -1) {
			this->mItemPaddingTop = this->mItemPaddingBottom = LoadAttrIntScaleY(child, "size", 0);
		}
	}
	
	SetMaxIconSize(iconWidth, iconHeight);

	// Handle the result variable
	child = FindNode(node, "data");
	if (child) {
		attr = child->first_attribute("requireReload");
		if (attr)
			requireReload = true;
		attr = child->first_attribute("name");
		if (attr)
			mVariable = attr->value();
		attr = child->first_attribute("default");
		if (attr)
			DataManager::SetValue(mVariable, attr->value());
		// Get the currently selected value for the list
		DataManager::GetValue(mVariable, currentValue);
		if (mVariable == "tw_language") {
			std::vector<language_struct>::iterator iter;
			for (iter = Language_List.begin(); iter != Language_List.end(); iter++) {
				ListItem data;
				data.displayName = (*iter).displayvalue;
				data.variableValue = (*iter).filename;
				data.action = NULL;
				if (currentValue == (*iter).filename) {
					data.icon = mIconSelected;
					DataManager::SetValue("tw_language_display", (*iter).displayvalue);
				} else
					data.icon = mIconUnselected;
				mListItems.push_back(data);
			}
		}
	} else
		allowSelection = false;  // allows using listbox as a read-only list or menu

	//[f/d] read file
	child = FindNode(node, "read");
	if (child) {
		attr = child->first_attribute("filename");
		if (attr) {
			mFileName = attr->value();
			std::vector<string> lines;
			if (TWFunc::read_file(mFileName.c_str(), lines) == 0) {
			  LOGINFO("Parsing file: %s\n", mFileName.c_str());
				unsigned int vector_size = lines.size();
				for (unsigned int i = 0; i < vector_size; i++) {
					ListItem item;
					item.displayName = lines[i];
					item.selected = false;
					item.action = NULL;
					item.hasicon = false;
					item.variableValue = "";

					mListItems.push_back(item);
					mVisibleItems.push_back(mListItems.size()-1);
				}
			} else {
				ListItem item;
				item.displayName = gui_parse_text("{@file_read_error=Unable to open file!}");
				item.selected = false;
				item.action = NULL;
				item.hasicon = false;
				item.variableValue = "";
				
				mListItems.push_back(item);
				mVisibleItems.push_back(mListItems.size()-1);
			}
			return;
		}
	}

	// Get the data for the list
	child = FindNode(node, "listitem");
	if (!child) return;
	while (child) {
		ListItem item;

		attr = child->first_attribute("name");
		if (!attr) continue;
		// We will parse display names when we get page focus to ensure that translating takes place
		item.displayName = attr->value();
		if (requireReload)
			item.unparsedName = attr->value();
		item.variableValue = gui_parse_text(child->value());
		item.selected = (child->value() == currentValue);
		item.action = NULL;
		xml_node<>* action = child->first_node("action");
		if (!action) action = child->first_node("actions");
		if (action) {
			item.action = new GUIAction(child);
			allowSelection = true;
		}
		
		// [f/d] Load custom icon
		xml_node<>* exicon = child->first_node("icon");
		if (exicon) {
			item.icon = LoadAttrImage(exicon, "res");
			item.hasicon = true;
		} else {
			item.hasicon = false;
		}
		
		xml_node<>* variable_name = child->first_node("data");
		if (variable_name) {
			attr = variable_name->first_attribute("variable");
			if (attr) {
				item.variableName = attr->value();
				item.selected = (DataManager::GetIntValue(item.variableName) != 0);
				allowSelection = true;
				isCheckList = true;
			}
		}

		LoadConditions(child, item.mConditions);

		mListItems.push_back(item);
		mVisibleItems.push_back(mListItems.size() - 1);

		child = child->next_sibling("listitem");
	}

	child = FindNode(node, "group");
	if (child) {
		groupArrow = LoadAttrImage(child, "arrowresource");
		groupColor = LoadAttrColor(child, "color", &isGroup);
	}
}

GUIListBox::~GUIListBox()
{
}

//[f/d] this function is called only on update.
//In TWRP it also called at init, but actually it's useless.
//If you'll see empty users list, add fuction call to init
void GUIListBox::CreateEncryptUsersList(void) {
	mListItems.clear();
	mVisibleItems.clear();
	std::vector<users_struct>::iterator iter;
	std::vector<users_struct>* Users_List = PartitionManager.Get_Users_List();
	unsigned int id = 0;
	for (iter = Users_List->begin(); iter != Users_List->end(); iter++) {
		if (!(*iter).isDecrypted) {
			ListItem data;
			data.displayName = (*iter).userName;
			data.variableValue = (*iter).userId;
			data.id = to_string((*iter).type);
			data.action = NULL;
			data.icon = mIconSelected;
			data.selected = 0;
			mListItems.push_back(data);
			mVisibleItems.push_back(id);
		}
		id++;
	}
}

//[f/d]
void GUIListBox::ReadFileToList(const char* fileName) {
	gui_msg(Msg(msg::kNormal, "file_read=Reading file: {1}")(fileName));
	mListItems.clear();
	mVisibleItems.clear();
	SetVisibleListLocation(0);
	string error = "Error";
	std::vector<wstring> lines;

	lines.push_back(L"");
	
	if (TWFunc::Get_File_Size(fileName) > 1572864) //1.5mb
		error = gui_parse_text("{@file_read_error_size=File is bigger than 1.5MB!}");
	else if (TWFunc::read_file(fileName, lines) == 0) {
		if ((lines[0] + lines[1]).find('\0') != std::string::npos) // i
			error = gui_parse_text("{@file_read_error_bin=Can't read binary file!}");
		else {
			lines.push_back(L"");
			unsigned int vector_size = lines.size();
			for (unsigned int i = 0; i < vector_size; i++) {
				wstring line = lines[i];
				size_t len = line.length();
				
				if (len <= 54) {
					ListItem item;
					item.displayName = TWFunc::wstr_to_str(line);
					item.variableValue = "";
					item.selected = 1;
					item.action = NULL;
					item.hasicon = false;

					mListItems.push_back(item);
					mVisibleItems.push_back(mListItems.size()-1);
				} else {
					size_t off = 0;
					do {
						ListItem item;
						
						item.displayName = TWFunc::wstr_to_str(line.substr(off, 54));
						item.variableValue = "";
						item.selected = 0;
						item.action = NULL;
						item.hasicon = false;

						mListItems.push_back(item);
						mVisibleItems.push_back(mListItems.size()-1);
						off += 54;
					} while (off < len);
				}
			}
  			gui_msg("done=Done.");
			return;
		}
	} else
		error = gui_parse_text("{@file_read_error=Unable to open file!}");
		
	for (int i = 0; i < 2; i++)
	{
		ListItem item;
		item.displayName = i == 1 ? error : "";
		item.selected = 0;
		item.action = NULL;
		item.hasicon = false;
		item.variableValue = "";
		
		mListItems.push_back(item);
		mVisibleItems.push_back(mListItems.size()-1);
	}
	

	gui_print_color("warning", "%s\n", error.c_str());
}

int GUIListBox::Update(void)
{
	if (!isConditionTrue())
		return 0;

	GUIScrollList::Update();

	if (mUpdate) {
		mUpdate = 0;
		if (Render() == 0)
			return 2;
	}
	return 0;
}

int GUIListBox::NotifyVarChange(const std::string& varName, const std::string& value)
{
	GUIScrollList::NotifyVarChange(varName, value);

	if (!isConditionTrue())
		return 0;

	// Check to see if the variable that we are using to store the list selected value has been updated
	if (varName == mVariable) {
		if (mVariable == "tw_crypto_user_id_list" &&
			DataManager::GetStrValue("tw_crypto_user_id_list") == "") //don't update on click 
			CreateEncryptUsersList();

		if (mVariable == "of_file_to_read" && currentValue != value) {
			if (value == "") {
				mListItems.clear(); //free memory or something
				mVisibleItems.clear();
			} else
				ReadFileToList(value.c_str());
		}

		currentValue = value;
		mUpdate = 1;
	}

	std::vector<size_t> mVisibleItemsOld;
	std::swap(mVisibleItemsOld, mVisibleItems);
	for (size_t i = 0; i < mListItems.size(); i++) {
		ListItem& item = mListItems[i];
		// update per-item visibility condition
		bool itemVisible = UpdateConditions(item.mConditions, varName);
		if (itemVisible)
			mVisibleItems.push_back(i);

		if (requireReload)
			item.displayName = gui_parse_text(item.unparsedName);

		if (isCheckList)
		{
			if (item.variableName == varName || varName.empty()) {
				std::string val;
				DataManager::GetValue(item.variableName, val);
				item.selected = (val != "0");
				mUpdate = 1;
			}
		}
		else if (varName == mVariable) {
			if (item.variableValue == currentValue) {
				item.selected = 1;
				SetVisibleListLocation(mVisibleItems.empty() ? 0 : mVisibleItems.size()-1);
			} else {
				item.selected = 0;
			}
		}
	}

	if (mVisibleItemsOld != mVisibleItems) {
		mUpdate = 1; // some item's visibility has changed
		if (firstDisplayedItem >= (int)mVisibleItems.size()) {
			// all items in the view area were removed - make last item visible
			SetVisibleListLocation(mVisibleItems.empty() ? 0 : mVisibleItems.size()-1);
		}
	}

	return 0;
}

void GUIListBox::SetPageFocus(int inFocus)
{
	GUIScrollList::SetPageFocus(inFocus);
	if (inFocus) {
		if (!isTextParsed) {
			isTextParsed = true;
			for (size_t i = 0; i < mListItems.size(); i++) {
				ListItem& item = mListItems[i];
				item.displayName = gui_parse_text(item.displayName);
			}
		}
		DataManager::GetValue(mVariable, currentValue);
		NotifyVarChange(mVariable, currentValue);
	}
}

size_t GUIListBox::GetItemCount() const
{
	return mVisibleItems.size();
}

void GUIListBox::RenderItem(size_t itemindex, int yPos, bool selected)
{
	if (itemindex >= mVisibleItems.size())
		return;

	// note: the "selected" parameter above is for the currently touched item
	// don't confuse it with the more persistent "selected" flag per list item used below
	ListItem& item = mListItems[mVisibleItems[itemindex]];
	ImageResource* icon;
	if (item.hasicon) {
		//[f/d] Render custom icon
		icon = item.icon;
	} else {
		//Render default (un)selected icon
		icon = item.selected ? mIconSelected : mIconUnselected;
	}
	const std::string& text = item.displayName;
	int groupStatus = 0;
	if (isGroup) {
		if (itemindex == 0)
			if (mVisibleItems.size() == 1)
				groupStatus = 4; // start & end
			else
				groupStatus = 1; // start
		else if (itemindex == mVisibleItems.size() - 1)
			groupStatus = 3; // end
		else
			groupStatus = 2; // body
	}

	RenderStdItem(yPos, selected, icon, text.c_str(), NULL, groupStatus);
}

void GUIListBox::NotifySelect(size_t item_selected)
{
	if (mVariable == "of_file_to_read") return;
	if (!isCheckList) {
		// deselect all items, even invisible ones
		for (size_t i = 0; i < mListItems.size(); i++) {
			mListItems[i].selected = 0;
		}
	}

	ListItem& item = mListItems[mVisibleItems[item_selected]];
	
	if (mVariable == "tw_crypto_user_id_list") {
		DataManager::SetValue("tw_crypto_user_display", item.displayName);
		DataManager::SetValue("tw_crypto_user_id", item.variableValue);
		DataManager::SetValue("tw_crypto_pwtype", item.id);
		DataManager::SetValue(mVariable, item.variableValue);
	} else if (isCheckList) {
		int selected = 1 - item.selected;
		item.selected = selected;
		DataManager::SetValue(item.variableName, selected ? "1" : "0");
	} else {
		item.selected = 1;
		string str = item.variableValue;	// [check] should this set currentValue instead?
		DataManager::SetValue(mVariable, str);
	}
	if (item.action)
		item.action->doActions();
	mUpdate = 1;
}
