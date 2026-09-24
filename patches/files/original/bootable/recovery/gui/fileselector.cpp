/*
	Copyright 2012 bigbiff/Dees_Troy TeamWin
	This file is part of TWRP/TeamWin Recovery Project.

	Copyright (C) 2018-2026 OrangeFox Recovery Project
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
#include <sys/stat.h>
#include <dirent.h>
#include <algorithm>
#ifdef __ANDROID_API_M__
#include <vector>
#ifdef __ANDROID_API_N__
#include <android-base/strings.h>
#else
#include <base/strings.h>
#endif
#else
#endif
extern "C" {
#include "../twcommon.h"
}
#include "minuitwrp/minui.h"

#include "rapidxml.hpp"
#include "objects.hpp"
#include "../data.hpp"
#include "../twrp-functions.hpp"
#include "../adbbu/libtwadbbu.hpp"

int GUIFileSelector::mSortOrder = 0;

GUIFileSelector::GUIFileSelector(xml_node<>* node) : GUIScrollList(node)
{
	xml_attribute<>* attr;
	xml_node<>* child;

	mFolderIcon = mFileIcon = mUpIcon = mExZipIcon = mExImgIcon = mExTxtIcon = mExUnselectedIcon = mExSelectedIcon = mExPngIcon = mExLinkIcon = mExBlockIcon = NULL;
	mIconBg = NULL;
	mShowFolders = mShowFiles = mShowNavFolders = 1;
	mUpdate = 0;
	mPathVar = "cwd";
	mFileFilterVar = "";
	ignoreHideVar = updateFileList = false;
	allowDouble = mSelListEnabled = hasFiles = hasHiddenFiles = false;

	// Load filter for filtering files (e.g. *.zip for only zips)
	child = FindNode(node, "filter");
	if (child) {
		// [f/d] use variable as extension filter (if extnvar not found use classic extn)
		attr = child->first_attribute("extnvar");
		if (attr) {
			mExtnVar = attr->value();
			DataManager::GetValue(mExtnVar, mExtn);
		} else {
			attr = child->first_attribute("extn");
			if (attr)
				mExtn= attr->value();
		}

		attr = child->first_attribute("name");
		if (attr)
			mFileFilterVar = attr->value();
		attr = child->first_attribute("folders");
		if (attr)
			mShowFolders = atoi(attr->value());
		attr = child->first_attribute("files");
		if (attr)
			mShowFiles = atoi(attr->value());
		attr = child->first_attribute("nav");
		if (attr)
			mShowNavFolders = atoi(attr->value());
		attr = child->first_attribute("hidden");
		if (attr)
			ignoreHideVar = true;
	}

	// Handle the path variable
	child = FindNode(node, "path");
	if (child) {
		attr = child->first_attribute("name");
		if (attr)
			mPathVar = attr->value();
		attr = child->first_attribute("default");
		if (attr) {
			mPathDefault = attr->value();
			DataManager::SetValue(mPathVar, attr->value());
		}
	}

	// Handle the result variable
	child = FindNode(node, "data");
	if (child) {
		attr = child->first_attribute("name");
		if (attr)
			mVariable = attr->value();
		attr = child->first_attribute("default");
		if (attr)
			DataManager::SetValue(mVariable, attr->value());
	}

	// Handle the sort variable
	child = FindNode(node, "sort");
	if (child) {
		attr = child->first_attribute("name");
		if (attr)
			mSortVariable = attr->value();
		attr = child->first_attribute("default");
		if (attr)
			DataManager::SetValue(mSortVariable, attr->value());

		DataManager::GetValue(mSortVariable, mSortOrder);
	}

	// Handle the selection variable
	child = FindNode(node, "selection");
	if (child && (attr = child->first_attribute("name")))
		mSelection = attr->value();
	else
		mSelection = "0";

	// [f/d] Multiselection
	child = FindNode(node, "extra");
	if (child) {
		if (child->first_attribute("multi"))
			mSelListEnabled = true;
		if (child->first_attribute("double"))
			allowDouble = true;
	}
	
	// Get folder and file icons if present
	child = FindNode(node, "icon");
	if (child) {
		mFolderIcon = LoadAttrImage(child, "folder");
		mFileIcon   = LoadAttrImage(child, "file");
	}
	
	// [f/d] Use file & folder icons for add. icons when exicon node not found
	mExZipIcon = mExImgIcon = mExTxtIcon = mExLinkIcon = mExPngIcon = mFileIcon;
	mUpIcon = mFolderIcon;
	
	int iconWidth = 0, iconHeight = 0;
	
	// [f/d] Get size for icons
	// [f/d] UPD. exicons only availble if iconsize is set. 
	//            fox wont load
	child = FindNode(node, "iconsize");
	if (child) {
		iconWidth = LoadAttrIntScaleX(child, "w", iconWidth);
		iconHeight = LoadAttrIntScaleY(child, "h", iconHeight);
		// Optional layered icon background when iconsize is set
		mIconBg = LoadAttrImage(child, "bg");
		
		// [f/d] Get additional icons
		child = FindNode(node, "exicon");
		if (child) {
			mExZipIcon   = LoadAttrImage(child, "zip");
			mExImgIcon   = LoadAttrImage(child, "img");
			mExTxtIcon   = LoadAttrImage(child, "txt");
			mExPngIcon   = LoadAttrImage(child, "png");
			mExLinkIcon  = LoadAttrImage(child, "link");
			mExBlockIcon = LoadAttrImage(child, "block");
			mUpIcon      = LoadAttrImage(child, "up");
			mExSelectedIcon = LoadAttrImage(child, "select");
			mExUnselectedIcon = LoadAttrImage(child, "unselect");
		}
	} else {
		if (mFolderIcon && mFolderIcon->GetResource() && mFileIcon && mFileIcon->GetResource()) {
			iconWidth = std::max(mFolderIcon->GetWidth(), mFileIcon->GetWidth());
			iconHeight = std::max(mFolderIcon->GetHeight(), mFileIcon->GetHeight());
		} else if (mFolderIcon && mFolderIcon->GetResource()) {
			iconWidth = mFolderIcon->GetWidth();
			iconHeight = mFolderIcon->GetHeight();
		} else if (mFileIcon && mFileIcon->GetResource()) {
			iconWidth = mFileIcon->GetWidth();
			iconHeight = mFileIcon->GetHeight();
		}
	}
	
	SetMaxIconSize(iconWidth, iconHeight);

	// Fetch the file/folder list
	std::string value;
	DataManager::GetValue(mPathVar, value);
	GetFileList(value);
}

GUIFileSelector::~GUIFileSelector()
{
}

int GUIFileSelector::Update(void)
{
	if (!isConditionTrue())
		return 0;

	GUIScrollList::Update();

	// Update the file list if needed
	if (updateFileList) {
		string value;
		DataManager::GetValue(mPathVar, value);
		if (GetFileList(value) == 0) {
			updateFileList = false;
			mUpdate = 1;
		} else
			return 0;
	}

	if (mUpdate) {
		mUpdate = 0;
		if (Render() == 0)
			return 2;
	}
	return 0;
}

int GUIFileSelector::NotifyVarChange(const std::string& varName, const std::string& value)
{
	GUIScrollList::NotifyVarChange(varName, value);

	if (!isConditionTrue())
		return 0;

	if (varName.empty()) {
		// Always clear the data variable so we know to use it
		DataManager::SetValue(mVariable, "");
	}
	if (varName == mPathVar || varName == mSortVariable || varName == mExtnVar) {
		if (varName == mSortVariable) {
			DataManager::GetValue(mSortVariable, mSortOrder);
		} else if (varName == mExtnVar) {
			DataManager::GetValue(mExtnVar, mExtn);
			SetVisibleListLocation(0);
		} else {
			// Reset the list to the top
			SetVisibleListLocation(0);
			if (value.empty())
				DataManager::SetValue(mPathVar, mPathDefault);
		}
		updateFileList = true;
		mUpdate = 1;
		return 0;
	}
	return 0;
}

bool GUIFileSelector::fileSort(FileData d1, FileData d2)
{
	if (d1.fileName == ".")
		return -1;
	if (d2.fileName == ".")
		return 0;
	if (d1.fileName == "..")
		return -1;
	if (d2.fileName == "..")
		return 0;

	switch (mSortOrder) {
		case 3: // by size largest first
			if (d1.fileSize == d2.fileSize || d1.fileType == DT_DIR) // some directories report a different size than others - but this is not the size of the files inside the directory, so we just sort by name on directories
				return (strcasecmp(d1.fileName.c_str(), d2.fileName.c_str()) < 0);
			return d1.fileSize < d2.fileSize;
		case -3: // by size smallest first
			if (d1.fileSize == d2.fileSize || d1.fileType == DT_DIR) // some directories report a different size than others - but this is not the size of the files inside the directory, so we just sort by name on directories
				return (strcasecmp(d1.fileName.c_str(), d2.fileName.c_str()) > 0);
			return d1.fileSize > d2.fileSize;
		case 2: // by last modified date newest first
			if (d1.lastModified == d2.lastModified)
				return (strcasecmp(d1.fileName.c_str(), d2.fileName.c_str()) < 0);
			return d1.lastModified < d2.lastModified;
		case -2: // by date oldest first
			if (d1.lastModified == d2.lastModified)
				return (strcasecmp(d1.fileName.c_str(), d2.fileName.c_str()) > 0);
			return d1.lastModified > d2.lastModified;
		case -1: // by name descending
			return (strcasecmp(d1.fileName.c_str(), d2.fileName.c_str()) > 0);
		default: // should be a 1 - sort by name ascending
			return (strcasecmp(d1.fileName.c_str(), d2.fileName.c_str()) < 0);
	}
	return 0;
}

int GUIFileSelector::GetFileList(const std::string folder)
{
	DIR* d;
	struct dirent* de;
	struct stat st;

	hasHiddenFiles = false;
	hasFiles = false;

	// Clear all data
	mFolderList.clear();
	mFileList.clear();

	d = opendir(folder.c_str());
	if (d == NULL) {
		LOGINFO("Unable to open '%s'\n", folder.c_str());
		if (folder != "/" && (mShowNavFolders != 0 || mShowFiles != 0)) {
			size_t found;
			found = folder.find_last_of('/');
			if (found != string::npos) {
				string new_folder = folder.substr(0, found);

				if (new_folder.length() < 2)
					new_folder = "/";
				DataManager::SetValue(mPathVar, new_folder);
			}
		}
		return -1;
	}

	if (allowDouble)
		DataManager::GetValue("list_font", doubleLine);
	
	string reloadfm, searchString, showHiddenFiles;
	if (mFileFilterVar != "") {
		searchString = TWFunc::lowercase(DataManager::GetStrValue(mFileFilterVar));
		showHiddenFiles = "1";
	} else
		if (ignoreHideVar)
			showHiddenFiles = "0";
		else
			DataManager::GetValue("tw_hidden_files", showHiddenFiles);
	DataManager::GetValue("tw_reload_fm", reloadfm);
	if (reloadfm == "1") {
		SetVisibleListLocation(0); // Scrolls to top
		DataManager::SetValue("tw_reload_fm", "0");
	}
	
	while ((de = readdir(d)) != NULL) {
		FileData data;

		data.fileName = de->d_name;
		if (data.fileName == ".")
			continue;
		if (data.fileName == ".." && folder == "/")
			continue;
		
		// [f/d] filter files by name
		if (searchString != "" && mFileFilterVar != "") {
			if (data.fileName != ".." && data.fileName.find(searchString) == string::npos){
				string fileLower = TWFunc::lowercase(data.fileName);
				if (fileLower.find(searchString) == string::npos)
					continue;
			}
		}
		
		// [f/d] Remove hidden files/folders when tw_hidden_files = 0
		if (showHiddenFiles == "0") {
			if ( (folder == "/" && (data.fileName == "twres" || data.fileName == "tmp"))
			||   (data.fileName != ".." && data.fileName.substr(0, 1) == ".")
			||    data.fileName == "lost+found" ) {
				hasHiddenFiles = true;
				continue;
			}
		}
		
		if (data.fileName != "..")
			hasFiles = true;
		
		data.fileType = de->d_type;

		std::string path = folder + "/" + data.fileName;
		stat(path.c_str(), &st);
		data.protection = st.st_mode;
		data.userId = st.st_uid;
		data.groupId = st.st_gid;
		data.fileSize = st.st_size;
		data.lastAccess = st.st_atime;
		data.lastModified = st.st_mtime;
		data.lastStatChange = st.st_ctime;

		if (data.fileType == DT_UNKNOWN) {
			data.fileType = TWFunc::Get_D_Type_From_Stat(path);
		}
		if (data.fileType == DT_DIR) {
			if (mShowNavFolders || (data.fileName != "." && data.fileName != ".."))
				mFolderList.push_back(data);
		} else if (data.fileType == DT_REG || data.fileType == DT_LNK || data.fileType == DT_BLK) {
#ifdef __ANDROID_API_M__
			std::vector<std::string> mExtnResults = android::base::Split(mExtn, ";");
			for (const std::string& mExtnElement : mExtnResults)
			{
				std::string mExtnName = android::base::Trim(mExtnElement);
				if (mExtnName.empty() || (data.fileName.length() > mExtnName.length() && TWFunc::lowercase(data.fileName.substr(data.fileName.length() - mExtnName.length())) == mExtnName)) {
					if (mExtnName == ".ab" && twadbbu::Check_ADB_Backup_File(path)) {
						mFolderList.push_back(data);
					} else {
						// [f/d] Get file extension
						data.fileExt = TWFunc::lowercase(data.fileName.substr(data.fileName.find_last_of(".") + 1));
						mFileList.push_back(data);
					}
				}
			}
#else //On android 5.1 we can't use android::base::Trim and Split so just use the first extension written in the list
			std::size_t seppos = mExtn.find_first_of(";");
			std::string mExtnf;
			if (seppos!=std::string::npos){
				mExtnf = mExtn.substr(0, seppos);
			} else {
				mExtnf = mExtn;
			}
			if (mExtnf.empty() || (data.fileName.length() > mExtnf.length() && TWFunc::lowercase(data.fileName.substr(data.fileName.length() - mExtnf.length())) == mExtnf)) {
				if (mExtnf == ".ab" && twadbbu::Check_ADB_Backup_File(path)) {
					mFolderList.push_back(data);
				} else {
					// [f/d] Get file extension
					data.fileExt = TWFunc::lowercase(data.fileName.substr(data.fileName.find_last_of(".") + 1));
					mFileList.push_back(data);
				}
			}
#endif
		}
 	}
	closedir(d);

	std::sort(mFolderList.begin(), mFolderList.end(), fileSort);
	std::sort(mFileList.begin(), mFileList.end(), fileSort);

	DataManager::SetValue("of_empty_dir", hasFiles ? 0 : hasHiddenFiles ? 2 : 1);

	return 0;
}

void GUIFileSelector::SetPageFocus(int inFocus)
{
	GUIScrollList::SetPageFocus(inFocus);
	if (inFocus) {
		std::string value;
		DataManager::GetValue(mPathVar, value);
		if (value.empty())
			DataManager::SetValue(mPathVar, mPathDefault);
		updateFileList = true;
		mUpdate = 1;
	}
}

size_t GUIFileSelector::GetItemCount() const
{
	size_t folderSize = mShowFolders ? mFolderList.size() : 0;
	size_t fileSize = mShowFiles ? mFileList.size() : 0;
	return folderSize + fileSize;
}

void GUIFileSelector::RenderItem(size_t itemindex, int yPos, bool selected)
{
	size_t folderSize = mShowFolders ? mFolderList.size() : 0;
	size_t fileindex = itemindex - folderSize;
	string secondLine = "";
	
	ImageResource* icon;
	std::string text, ext;
	unsigned char type;

	if (itemindex < folderSize) {
		text = mFolderList.at(itemindex).fileName;
		if (text == "..") {
			text = gui_lookup("up_a_level", "(Up A Level)");
			icon = mUpIcon;
		} else {
			if (allowDouble && doubleLine == 1)
				secondLine = TWFunc::ConvertTime(mFolderList.at(itemindex).lastModified);
			if (mSelListEnabled) {
				std::string list = DataManager::GetStrValue("of_batch_folders");
				// list.find(text + "/") != string::npos
				if (list.find("/" + text + "/") != string::npos || list.rfind(text + "/", 0) == 0) // prevents situation when 'Dxxx/' = 'xxx/'
					icon = mExSelectedIcon;
				else
					icon = mExUnselectedIcon;
			} else
				icon = mFolderIcon;
		}
	} else {
		text = mFileList.at(fileindex).fileName;
		if (allowDouble && doubleLine == 1) {
			secondLine = TWFunc::ConvertTime(mFileList.at(fileindex).lastModified) + " · " + to_string(mFileList.at(fileindex).fileSize / 1024) + gui_parse_text("{@kbyte}");
//			secondLine = TWFunc::ConvertTime(mFileList.at(fileindex).lastModified) + " · " + to_string(mFileList.at(fileindex).fileSize / 1048576) + gui_parse_text("{@mbyte}");
		}
		if (mSelListEnabled) {
			std::string list = DataManager::GetStrValue("of_batch_files");
			// list.find(text + "/") != string::npos
			if (list.find("/" + text + "/") != string::npos || list.rfind(text + "/", 0) == 0) // prevents situation when 'Dxxx/' = 'xxx/'
				icon = mExSelectedIcon;
			else
				icon = mExUnselectedIcon;
		} else {
			ext  = mFileList.at(fileindex).fileExt;
			type = mFileList.at(fileindex).fileType;
			
			// [f/d] Detect symlink
			if (type == DT_LNK) {
				icon = mExLinkIcon;
			} else if (type == DT_BLK || type == DT_CHR) {
				icon = mExBlockIcon;
			} else {
				// [f/d] Detect file extension and set icon
				if (ext == "zip" || ext == "apk" || ext == "tar" || ext == "gz" || ext == "bz2" || ext == "xz" || ext == "lzo" || ext == "cpio" || ext == "lzma" || ext == "z" || ext == "zz") {
					icon = mExZipIcon;
				} else if (ext == "img" || ext == "win") {
					icon = mExImgIcon;
				} else if (ext == "png" || ext == "jpg" || ext == "bmp" || ext == "gif") {
					icon = mExPngIcon;
				} else if (ext == "txt" || ext == "log"  || ext == "json" || ext == "cfg" || ext == "prop" || ext == "xml" || ext == "sh" || ext == "rc" || ext == "conf" || ext == "fstab" || ext == "default") {
					icon = mExTxtIcon;
				} 
				#ifdef OF_SUPPORT_OZIP_DECRYPTION
					else if (ext == "ozip") {
						icon = mExZipIcon;
					}
				#endif
				else {
					icon = mFileIcon;
				}
			}
		}
	}

 	// TODO: add useful info like: 15:44 25.06.2019 23MB (755)
	// mFileList.at(fileindex).fileSize
	// mFileList.at(fileindex).lastAccess
	// mFileList.at(fileindex).lastModified
	// mFileList.at(fileindex).lastStatChange

	// If layered background icon is available and iconsize is defined, render in two layers
	if (mIconBg && mIconBg->GetResource()) {
		// First render the row without an icon so text/layout is correct
		if (allowDouble && doubleLine == 1 && secondLine != "")
			RenderStdItem(yPos, selected, NULL, text.c_str(), secondLine.c_str());
		else
			RenderStdItem(yPos, selected, NULL, text.c_str());
		// Then draw background circle and the foreground icon centered in icon area
		int iconAndTextH = actualItemHeight;
		int bgW = mIconBg->GetWidth();
		int bgH = mIconBg->GetHeight();
		int bgX = mRenderX + (maxIconWidth - bgW) / 2 - mPadding;
		int bgY = yPos + (iconAndTextH - bgH) / 2;
		gr_blit(mIconBg->GetResource(), 0, 0, bgW, bgH, bgX, bgY);
		if (icon && icon->GetResource()) {
			int iw = icon->GetWidth();
			int ih = icon->GetHeight();
			int ix = mRenderX + (maxIconWidth - iw) / 2 - mPadding;
			int iy = yPos + (iconAndTextH - ih) / 2;
			gr_blit(icon->GetResource(), 0, 0, iw, ih, ix, iy);
		}
	} else {
		if (allowDouble && doubleLine == 1 && secondLine != "")
			RenderStdItem(yPos, selected, icon, text.c_str(), secondLine.c_str());
		else
			RenderStdItem(yPos, selected, icon, text.c_str());
	}
}

void GUIFileSelector::NotifySelect(size_t item_selected)
{
	size_t folderSize = mShowFolders ? mFolderList.size() : 0;
	size_t fileSize = mShowFiles ? mFileList.size() : 0;
	std::string msListVar;

	if (item_selected < folderSize + fileSize) {
		// We've selected an item!
		std::string str;
		
		// Resetting vars
		DataManager::SetValue("tw_real_path", "");
		DataManager::SetValue("tw_rp_type", "3");
		
		if (item_selected < folderSize) {
			// Path selection
			std::string cwd;
			msListVar = "of_batch_folders";

			str = mFolderList.at(item_selected).fileName;
			if (mSelection != "0")
				DataManager::SetValue(mSelection, str);
			DataManager::GetValue(mPathVar, cwd);

			// Ignore requests to do nothing
			if (str == ".")	 return;
			if (str == "..") {
				if (cwd != "/") {
					size_t found;
					found = cwd.find_last_of('/');
					cwd = cwd.substr(0,found);

					if (cwd.length() < 2)   cwd = "/";
				}
			} else {
				// Add a slash if we're not the root folder
				if (cwd != "/")	 cwd += "/";
				cwd += str;
			}
			
			DataManager::SetValue("tw_fm_isfolder", 1);
			
			DataManager::GetValue(itemHold, itemHldStatus);
			if (itemHldStatus == "1") {
				DataManager::SetValue(mVariable, cwd);
			} else if (mShowNavFolders == 0 && (mShowFiles == 0 || mExtn == ".ab")) {
				// this is probably the restore list and we need to save chosen location to mVariable instead of mPathVar
				DataManager::SetValue(mVariable, cwd);
			} else if (!mSelListEnabled) {
				// We are changing paths, so we need to set mPathVar
				DataManager::SetValue(mPathVar, cwd);
			}
		} else if (!mVariable.empty()) {
			// File selection (data)
			str = mFileList.at(item_selected - folderSize).fileName;
			msListVar = "of_batch_files";
			if (mSelection != "0")
				DataManager::SetValue(mSelection, str);

			std::string cwd;
			DataManager::GetValue(mPathVar, cwd);
			if (cwd != "/")
				cwd += "/";
			
			std::string path = cwd + str;
		
			if (mFileList.at(item_selected - folderSize).fileType == DT_LNK) {
				// [f/d] We selected a symlink; Trying to get original file name
				char *real_path = realpath(path.c_str(), NULL);
				if (real_path) {
					// Is that dir or file
					std::string str_path = real_path;
					struct stat path_stat;
					stat(real_path, &path_stat);
					DataManager::SetValue("tw_real_path", str_path);
					DataManager::SetValue("tw_rp_type", S_ISDIR(path_stat.st_mode));
				} else {
					DataManager::SetValue("tw_rp_type", "2");
				}
				free(real_path);
			}
			DataManager::SetValue("tw_fm_isfolder", 0);
			DataManager::SetValue(mVariable, path);
		}

		if (mSelListEnabled && (!mVariable.empty() || item_selected < folderSize)) {
			// [f/d] Multiselection
			// We get something like this: Android/OFox.zip/DCIM/Pictures
			if (str == "..") return;
			std::string multiList = DataManager::GetStrValue(msListVar),
						fname = str + "/";
			size_t fnamepos = multiList.find(fname);

			DataManager::SetValue(msListVar, fnamepos != string::npos ?
			multiList.replace(fnamepos, fname.length(), "") : multiList + fname);

			int count = DataManager::GetIntValue("of_batch_count");
			DataManager::SetValue("of_batch_count", fnamepos != string::npos ?
			count - 1 : count + 1);
		}

	}
	mUpdate = 1;
}
