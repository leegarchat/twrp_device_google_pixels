/*
	Copyright 2013 bigbiff/Dees_Troy TeamWin
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

// pages.cpp - Source to manage GUI base objects

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>
#include <stdlib.h>
#include <dirent.h>
#include "../twrp-functions.hpp"
#include "../partitions.hpp"
#include "../variables.h"

#include <string>
#include <algorithm>
#include <map>
#include <mutex>
#include <vector>
#include <functional>


#include <ziparchive/zip_archive.h>
#include "ZipUtil.h"

extern "C" {
#include "../twcommon.h"
#include "gui.h"
}
#include "minuitwrp/minui.h"

#include "rapidxml.hpp"
#include "objects.hpp"
#include "blanktimer.hpp"

// version 2 requires theme to handle power button as action togglebacklight
#define TW_THEME_VERSION 3

#define TW_THEME_VER_ERR -2

extern int gGuiRunning;
GUITerminal* term = NULL;

std::map<std::string, PageSet*> PageManager::mPageSets;
PageSet* PageManager::mCurrentSet;
MouseCursor *PageManager::mMouseCursor = NULL;
HardwareKeyboard *PageManager::mHardwareKeyboard = NULL;
bool PageManager::mReloadTheme = false;
std::string PageManager::mStartPage = "main";
std::vector<language_struct> Language_List;
long mime;

int tw_x_offset = 0;
int tw_y_offset = 0;
int tw_w_offset = 0;
int tw_h_offset = 0;

// Helper routine to convert a string to a color declaration
int ConvertStrToColor(std::string str, COLOR* color)
{
	// Set the default, solid black
	memset(color, 0, sizeof(COLOR));
	color->alpha = 255;

	// Translate variables
	DataManager::GetValue(str, str);

	// Look for some defaults
	if (str == "black")		return 0;
	else if (str == "white")	{ color->red = color->green = color->blue = 255; return 0; }
	else if (str == "red")		{ color->red = 255; return 0; }
	else if (str == "green")	{ color->green = 255; return 0; }
	else if (str == "blue")		{ color->blue = 255; return 0; }
	else if (str == "pink")		{ color->red = 255; color->green = 192; color->blue = 203; return 0; }
	else if (str == "yellow")	{ color->red = color->green = 255; color->blue = 0; return 0; }

	// At this point, we require an RGB(A) color
	if (str[0] != '#')
		return -1;

	str.erase(0, 1);

	int result;
	if (str.size() >= 8) {
		// We have alpha channel
		string alpha = str.substr(6, 2);
		result = strtol(alpha.c_str(), NULL, 16);
		color->alpha = result & 0x000000FF;
		str.resize(6);
		result = strtol(str.c_str(), NULL, 16);
		color->red = (result >> 16) & 0x000000FF;
		color->green = (result >> 8) & 0x000000FF;
		color->blue = result & 0x000000FF;
	} else {
		result = strtol(str.c_str(), NULL, 16);
		color->red = (result >> 16) & 0x000000FF;
		color->green = (result >> 8) & 0x000000FF;
		color->blue = result & 0x000000FF;
	}
	return 0;
}

// Look up `nodename` in style `name`, following an `extends="base"` chain so a
// style can inherit a base style's nodes (the derived style wins). Bounded loop
// guards against extends cycles. Returns NULL if not found anywhere in the chain.
static xml_node<>* FindStyleChainNode(const std::string& name, const char* nodename, int depth)
{
	std::string cur = name;
	for (int guard = 0; !cur.empty() && guard < 12; guard++) {
		xml_node<>* node = PageManager::FindStyle(cur);
		if (!node)
			return NULL;
		xml_node<>* stylenode = FindNode(node, nodename, depth + 1);
		if (stylenode)
			return stylenode;
		xml_attribute<>* ext = node->first_attribute("extends");
		cur = ext ? ext->value() : "";
	}
	return NULL;
}

// Helper APIs
xml_node<>* FindNode(xml_node<>* parent, const char* nodename, int depth /* = 0 */)
{
	if (!parent)
		return NULL;

	xml_node<>* child = parent->first_node(nodename);
	if (child)
		return child;

	if (depth == 10) {
		LOGERR("Too many style loops detected.\n");
		return NULL;
	}

	xml_node<>* style = parent->first_node("style");
	if (style) {
		while (style) {
			if (!style->first_attribute("name")) {
				LOGERR("No name given for style.\n");
				continue;
			} else {
				std::string name = style->first_attribute("name")->value();
				xml_node<>* stylenode = FindStyleChainNode(name, nodename, depth);
				if (stylenode)
					return stylenode;
			}
			style = style->next_sibling("style");
		}
	} else {
		// Search for stylename in the parent node <object type="foo" style="foo2">
		xml_attribute<>* attr = parent->first_attribute("style");
		// If no style is found anywhere else and the node wasn't found in the object itself
		// as a special case we will search for a style that uses the same style name as the
		// object type, so <object type="button"> would search for a style named "button"
		if (!attr)
			attr = parent->first_attribute("type");
		// if there's no attribute type, the object type must be the element name
		std::string stylename = attr ? attr->value() : parent->name();
		xml_node<>* stylenode = FindStyleChainNode(stylename, nodename, depth);
		if (stylenode)
			return stylenode;
	}
	return NULL;
}

std::string LoadAttrString(xml_node<>* element, const char* attrname, const char* defaultvalue)
{
	if (!element)
		return defaultvalue;

	xml_attribute<>* attr = element->first_attribute(attrname);
	return attr ? attr->value() : defaultvalue;
}

int LoadAttrInt(xml_node<>* element, const char* attrname, int defaultvalue)
{
	string value = LoadAttrString(element, attrname);
	// resolve variables
	DataManager::GetValue(value, value);
	return value.empty() ? defaultvalue : atoi(value.c_str());
}

// A bare "NN%" literal (not a %var% reference) means a percentage of the
// framebuffer dimension. Returns true and fills *out when value is such a
// literal. "%foo%" (variable reference) starts with '%' and is left alone.
static bool ParsePercentDim(const std::string& value, bool isX, int* out)
{
	size_t n = value.size();
	if (n < 2 || value[0] == '%' || value[n - 1] != '%')
		return false;
	for (size_t i = 0; i + 1 < n; i++) {
		if (value[i] < '0' || value[i] > '9')
			return false;
	}
	int pct = atoi(value.c_str());
	*out = (isX ? gr_fb_width() : gr_fb_height()) * pct / 100;
	return true;
}

int LoadAttrIntScaleX(xml_node<>* element, const char* attrname, int defaultvalue)
{
	int pctval;
	if (ParsePercentDim(LoadAttrString(element, attrname), true, &pctval))
		return pctval;
	return scale_theme_x(LoadAttrInt(element, attrname, defaultvalue));
}

int LoadAttrIntScaleY(xml_node<>* element, const char* attrname, int defaultvalue)
{
	int pctval;
	if (ParsePercentDim(LoadAttrString(element, attrname), false, &pctval))
		return pctval;
	return scale_theme_y(LoadAttrInt(element, attrname, defaultvalue));
}

float LoadAttrFloat(xml_node<>* element, const char* attrname, float defaultvalue)
{
	string value = LoadAttrString(element, attrname);
	// resolve variables
	DataManager::GetValue(value, value);
	return value.empty() ? defaultvalue : stof(value.c_str());
}

COLOR LoadAttrColor(xml_node<>* element, const char* attrname, bool* found_color, COLOR defaultvalue)
{
	string value = LoadAttrString(element, attrname);
	*found_color = !value.empty();
	// resolve variables
	DataManager::GetValue(value, value);
	COLOR ret = defaultvalue;
	if (ConvertStrToColor(value, &ret) == 0)
		return ret;
	else
		return defaultvalue;
}

COLOR LoadAttrColor(xml_node<>* element, const char* attrname, COLOR defaultvalue)
{
	bool found_color = false;
	return LoadAttrColor(element, attrname, &found_color, defaultvalue);
}

FontResource* LoadAttrFont(xml_node<>* element, const char* attrname)
{
	std::string name = LoadAttrString(element, attrname, "");
	if (name.empty())
		return NULL;
	else
		return PageManager::GetResources()->FindFont(name);
}

ImageResource* LoadAttrImage(xml_node<>* element, const char* attrname)
{
	std::string name = LoadAttrString(element, attrname, "");
	if (name.empty())
		return NULL;
	else
		return PageManager::GetResources()->FindImage(name);
}

AnimationResource* LoadAttrAnimation(xml_node<>* element, const char* attrname)
{
	std::string name = LoadAttrString(element, attrname, "");
	if (name.empty())
		return NULL;
	else
		return PageManager::GetResources()->FindAnimation(name);
}

// ---- Relative / anchor placement (FoxUiEngine) ------------------------------
// Elements may carry id="name"; later elements anchor to them via:
//   y="below:name[+/-N]"   x="rightof:name[+/-N]"   (place after an edge)
//   y="top:name"  y="bottom:name"  x="left:name"  x="right:name"  (align edges)
// Resolved at load against rects recorded as objects are constructed in document
// order, so the referenced element must appear earlier on the page. Containers
// (column/row/scroll) reposition their children, so don't anchor into those.
struct LayoutRect { int x, y, w, h; };
static std::map<std::string, LayoutRect> g_layout_ids;

static bool ResolveRelativeDim(const std::string& value, bool isX, int* out)
{
	size_t colon = value.find(':');
	if (colon == std::string::npos)
		return false;
	std::string anchor = value.substr(0, colon);
	std::string rest = value.substr(colon + 1);

	int offset = 0;
	size_t sign = rest.find_first_of("+-");
	std::string id = rest;
	if (sign != std::string::npos) {
		id = rest.substr(0, sign);
		offset = atoi(rest.c_str() + sign);
	}

	std::map<std::string, LayoutRect>::iterator it = g_layout_ids.find(id);
	if (it == g_layout_ids.end())
		return false;
	const LayoutRect& r = it->second;

	if (!isX && anchor == "below")        *out = r.y + r.h;
	else if (!isX && anchor == "bottom")  *out = r.y + r.h;
	else if (!isX && anchor == "top")     *out = r.y;
	else if (isX && anchor == "rightof")  *out = r.x + r.w;
	else if (isX && anchor == "right")    *out = r.x + r.w;
	else if (isX && anchor == "left")     *out = r.x;
	else if (isX && anchor == "leftof")   *out = r.x;
	else
		return false;

	*out += isX ? scale_theme_x(offset) : scale_theme_y(offset);
	return true;
}

bool LoadPlacement(xml_node<>* node, int* x, int* y, int* w /* = NULL */, int* h /* = NULL */, Placement* placement /* = NULL */)
{
	if (!node)
		return false;

	if (node->first_attribute("x")) {
		if (!ResolveRelativeDim(LoadAttrString(node, "x"), true, x))
			*x = LoadAttrIntScaleX(node, "x") + tw_x_offset;
	}

	if (node->first_attribute("y")) {
		if (!ResolveRelativeDim(LoadAttrString(node, "y"), false, y))
			*y = LoadAttrIntScaleY(node, "y") + tw_y_offset;
	}

	// Safe-area inset: cutout="1" pushes the element down by the camera cutout
	// height (%cutout_w%) so authors don't fold it into every y by hand.
	if (node->first_attribute("cutout") && std::string(node->first_attribute("cutout")->value()) == "1") {
		int cut = 0;
		DataManager::GetValue("cutout_w", cut);
		*y += scale_theme_y(cut);
	}

	if (w && node->first_attribute("w"))
		*w = LoadAttrIntScaleX(node, "w");

	if (h && node->first_attribute("h"))
		*h = LoadAttrIntScaleY(node, "h");

	if (placement && node->first_attribute("placement"))
		*placement = (Placement) LoadAttrInt(node, "placement");

	return true;
}

int ActionObject::SetActionPos(int x, int y, int w, int h)
{
	if (x < 0 || y < 0)
		return -1;

	mActionX = x;
	mActionY = y;
	if (w || h)
	{
		mActionW = w;
		mActionH = h;
	}
	return 0;
}

Page::Page(xml_node<>* page, std::vector<xml_node<>*> *templates) : mFocusedObjectIndex(-1)
{
	mTouchStart = NULL;

	// We can memset the whole structure, because the alpha channel is ignored
	memset(&mBackground, 0, sizeof(COLOR));

	// With NULL, we make a console-only display
	if (!page)
	{
		mName = "console";

		GUIConsole* element = new GUIConsole(NULL);
		mRenders.push_back(element);
		mActions.push_back(element);
		return;
	}

	if (page->first_attribute("name"))
		mName = page->first_attribute("name")->value();
	else
	{
		LOGERR("No page name attribute found!\n");
		return;
	}

	LOGINFO("Loading page %s\n", mName.c_str());

	// This is a recursive routine for template handling
	ProcessNode(page, templates, 0);

	// Set focus on the first action
	if (!mActions.empty()) {
		SetFocus(0); // disable on hw_button_mode 0
	}
}

Page::~Page()
{
	for (std::vector<GUIObject*>::iterator itr = mObjects.begin(); itr != mObjects.end(); ++itr)
		delete *itr;
}

bool Page::ProcessNode(xml_node<>* page, std::vector<xml_node<>*> *templates, int depth)
{
	if (depth == 10)
	{
		LOGERR("Page processing depth has exceeded 10. Failing out. This is likely a recursive template.\n");
		return false;
	}

	// Fresh id->rect registry for relative/anchor placement on each page.
	if (depth == 0)
		g_layout_ids.clear();

	for (xml_node<>* child = page->first_node(); child; child = child->next_sibling())
	{
		std::string type = child->name();

		if (type == "background") {
			mBackground = LoadAttrColor(child, "color", COLOR(0,0,0,0));
			continue;
		}

		if (type == "object") {
			// legacy format : <object type="...">
			xml_attribute<>* attr = child->first_attribute("type");
			type = attr ? attr->value() : "*unspecified*";
		}

		// Meta nodes that may appear as children of a container element
		// (<scroll>/<column>/<row>) - they configure the parent, not objects.
		if (type == "placement" || type == "condition")
			continue;

		if (type == "text")
		{
			GUIText* element = new GUIText(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "gesture")
		{
			GUIGesture* element = new GUIGesture(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "battery")
		{
			GUIBattery* element = new GUIBattery(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
		}
		else if (type == "image")
		{
			GUIImage* element = new GUIImage(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
		}
		else if (type == "fill")
		{
			GUIFill* element = new GUIFill(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
		}
		else if (type == "spacer" || type == "divider")
		{
			GUIDivider* element = new GUIDivider(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
		}
		else if (type == "action")
		{
			GUIAction* element = new GUIAction(child);
			mObjects.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "console")
		{
			GUIConsole* element = new GUIConsole(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "terminal")
		{
			GUITerminal* element = new GUITerminal(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
			mInputs.push_back(element);
			term = element;
		}
		else if (type == "button")
		{
			GUIButton* element = new GUIButton(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "checkbox")
		{
			GUICheckbox* element = new GUICheckbox(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "toggle")
		{
			GUIToggle* element = new GUIToggle(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "fileselector")
		{
			GUIFileSelector* element = new GUIFileSelector(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "animation")
		{
			GUIAnimation* element = new GUIAnimation(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
		}
		else if (type == "spinningimage")
		{
			GUISpinningImage* element = new GUISpinningImage(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
		}
		else if (type == "progressbar")
		{
			GUIProgressBar* element = new GUIProgressBar(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "slider")
		{
			GUISlider* element = new GUISlider(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "slidervalue")
		{
			GUISliderValue *element = new GUISliderValue(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "listbox")
		{
			GUIListBox* element = new GUIListBox(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "keyboard")
		{
			GUIKeyboard* element = new GUIKeyboard(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "input")
		{
			GUIInput* element = new GUIInput(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
			mInputs.push_back(element);
		}
		else if (type == "partitionlist")
		{
			GUIPartitionList* element = new GUIPartitionList(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "patternpassword")
		{
			GUIPatternPassword* element = new GUIPatternPassword(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "textbox")
		{
			GUITextBox* element = new GUITextBox(child);
			mObjects.push_back(element);
			mRenders.push_back(element);
			mActions.push_back(element);
		}
		else if (type == "column" || type == "row")
		{
			// Auto-placement container (prototype). Children are laid out along
			// one axis from an origin, instead of each carrying an absolute
			// <placement>. The container itself is NOT a GUIObject: its children
			// are registered as normal page objects (so render order, focus and
			// touch all work via the existing Page machinery); we only rewrite
			// their positions after construction.
			//
			//   <column x="40" y="300" spacing="20" itemheight="60"> ... </column>
			//   <row    x="40" y="300" spacing="16" itemwidth="120"> ... </row>
			//
			// `direction` overrides the element default ("vertical" for column,
			// "horizontal" for row), so <column direction="horizontal"> works too.
			int origin_x = 0, origin_y = 0;
			if (child->first_attribute("x"))
				origin_x = LoadAttrIntScaleX(child, "x") + tw_x_offset;
			if (child->first_attribute("y"))
				origin_y = LoadAttrIntScaleY(child, "y") + tw_y_offset;

			bool horizontal = (type == "row");
			if (child->first_attribute("direction"))
				horizontal = (std::string(child->first_attribute("direction")->value()) == "horizontal");

			// `spacing` is the gap between items along the layout axis.
			int spacing = horizontal ? LoadAttrIntScaleX(child, "spacing", 0)
						 : LoadAttrIntScaleY(child, "spacing", 0);
			// Main-axis advance fallback for items with no intrinsic size.
			int defaultItemSize = horizontal ? LoadAttrIntScaleX(child, "itemwidth", 0)
							 : LoadAttrIntScaleY(child, "itemheight", 0);

			// Optional sizing for cross-axis align + main-axis weights:
			//   column: w = cross extent, h = main extent
			//   row:    h = cross extent, w = main extent
			int box_w = child->first_attribute("w") ? LoadAttrIntScaleX(child, "w", 0) : 0;
			int box_h = child->first_attribute("h") ? LoadAttrIntScaleY(child, "h", 0) : 0;
			int crossExtent = horizontal ? box_h : box_w;
			int mainExtent  = horizontal ? box_w : box_h;

			// align: start (default) | center | end (cross axis)
			int align = 0;
			if (child->first_attribute("align"))
			{
				std::string a = child->first_attribute("align")->value();
				if (a == "center")
					align = 1;
				else if (a == "end")
					align = 2;
			}

			// Per-child main-axis weights, in document order (meta nodes skipped
			// the same way ProcessNode skips them).
			std::vector<int> weights;
			for (xml_node<>* gc = child->first_node(); gc; gc = gc->next_sibling())
			{
				std::string gn = gc->name();
				if (gn == "placement" || gn == "condition")
					continue;
				weights.push_back(LoadAttrInt(gc, "weight", 0));
			}

			size_t firstObject = mObjects.size();
			if (!ProcessNode(child, templates, depth + 1))
				return false;
			LayoutContainer(firstObject, origin_x, origin_y, spacing, horizontal,
					defaultItemSize, crossExtent, align, mainExtent, weights);
		}
		else if (type == "card")
		{
			// A content-sizing rounded panel that OWNS its children (so it nests
			// and re-measures dynamic content): GUICard lays them out, sizes
			// itself to fit, and draws a rounded background + optional outline.
			// Same adoption as <scroll>: children stay in mObjects for lifetime /
			// var-change but are removed from page-level render/touch dispatch.
			GUICard* element = new GUICard(child);

			size_t firstObj = mObjects.size();
			size_t firstRender = mRenders.size();
			size_t firstAction = mActions.size();
			size_t firstInput = mInputs.size();

			if (!ProcessNode(child, templates, depth + 1))
				return false;

			for (size_t i = firstObj; i < mObjects.size(); i++)
			{
				GUIObject* obj = mObjects[i];
				element->Adopt(obj, dynamic_cast<RenderObject*>(obj), dynamic_cast<ActionObject*>(obj));
			}

			mRenders.erase(mRenders.begin() + firstRender, mRenders.end());
			mActions.erase(mActions.begin() + firstAction, mActions.end());
			mInputs.erase(mInputs.begin() + firstInput, mInputs.end());

			mRenders.insert(mRenders.begin() + firstRender, element);
			mActions.insert(mActions.begin() + firstAction, element);
			mObjects.push_back(element);
		}
		else if (type == "scroll")
		{
			// Scrollable viewport container. Like <column> it auto-places its
			// children vertically, but it OWNS them: it renders them clipped to
			// its viewport at a scroll offset and handles drag/fling. Build the
			// children as normal page objects, then hand them to the container
			// and remove them from the page-level render/touch dispatch so only
			// the container drives them. They stay in mObjects for lifetime and
			// var-change notification.
			GUIScrollContainer* element = new GUIScrollContainer(child);

			size_t firstObj = mObjects.size();
			size_t firstRender = mRenders.size();
			size_t firstAction = mActions.size();
			size_t firstInput = mInputs.size();

			if (!ProcessNode(child, templates, depth + 1))
				return false;

			for (size_t i = firstObj; i < mObjects.size(); i++)
			{
				GUIObject* obj = mObjects[i];
				element->Adopt(obj, dynamic_cast<RenderObject*>(obj), dynamic_cast<ActionObject*>(obj));
			}

			// Everything appended during the recursion above belongs to this
			// container, so the adopted ranges run to the end of each vector.
			mRenders.erase(mRenders.begin() + firstRender, mRenders.end());
			mActions.erase(mActions.begin() + firstAction, mActions.end());
			mInputs.erase(mInputs.begin() + firstInput, mInputs.end());

			// Slot the container into the z-position the children occupied.
			mRenders.insert(mRenders.begin() + firstRender, element);
			mActions.insert(mActions.begin() + firstAction, element);
			mObjects.push_back(element);
		}
		else if (type == "when")
		{
			// Responsive breakpoint: process children only if the current
			// framebuffer matches. Attributes (raw px): minw, maxw, minh, maxh,
			// and orientation="portrait|landscape".
			bool ok = true;
			if (child->first_attribute("minw"))
				ok = ok && (gr_fb_width() >= LoadAttrInt(child, "minw"));
			if (child->first_attribute("maxw"))
				ok = ok && (gr_fb_width() <= LoadAttrInt(child, "maxw"));
			if (child->first_attribute("minh"))
				ok = ok && (gr_fb_height() >= LoadAttrInt(child, "minh"));
			if (child->first_attribute("maxh"))
				ok = ok && (gr_fb_height() <= LoadAttrInt(child, "maxh"));
			if (child->first_attribute("orientation"))
			{
				bool landscape = gr_fb_width() > gr_fb_height();
				std::string o = child->first_attribute("orientation")->value();
				ok = ok && ((o == "landscape") == landscape);
			}
			if (ok && !ProcessNode(child, templates, depth + 1))
				return false;
		}
		else if (type == "template")
		{
			if (!templates || !child->first_attribute("name"))
			{
				LOGERR("Invalid template request.\n");
			}
			else
			{
				std::string name = child->first_attribute("name")->value();
				xml_node<>* node;
				bool node_found = false;

				// Parameterized templates: any attribute on the <template> call
				// other than "name" is a parameter exposed to the template body
				// as a %attr% DataManager value for the duration of expansion.
				// e.g. <template name="toggle" var="tw_x" label="ADB"/> lets the
				// template use %var% and %label%. Previous values are saved and
				// restored so params don't leak (supports nesting/reuse).
				std::vector<std::pair<std::string, std::string> > saved;
				for (xml_attribute<>* a = child->first_attribute(); a; a = a->next_attribute())
				{
					std::string an = a->name();
					if (an == "name")
						continue;
					saved.push_back(std::make_pair(an, DataManager::GetStrValue(an)));
					DataManager::SetValue(an, std::string(a->value()));
				}

				// We need to find the correct template
				for (std::vector<xml_node<>*>::iterator itr = templates->begin(); itr != templates->end(); itr++) {
					node = (*itr)->first_node("template");

					while (node)
					{
						if (!node->first_attribute("name"))
							continue;

						if (name == node->first_attribute("name")->value())
						{
							if (!ProcessNode(node, templates, depth + 1)) {
								for (size_t s = 0; s < saved.size(); s++)
									DataManager::SetValue(saved[s].first, saved[s].second);
								return false;
							}
							else {
								node_found = true;
								break;
							}
						}
						if (node_found)
							break;
						node = node->next_sibling("template");
					}
					// [check] why is there no if (node_found) here too?
				}

				// Restore any parameters we shadowed.
				for (size_t s = 0; s < saved.size(); s++)
					DataManager::SetValue(saved[s].first, saved[s].second);
			}
		}
		else
		{
			LOGERR("Unknown object type: %s.\n", type.c_str());
		}

		// Load-time validation: an explicit style="X" that doesn't resolve is
		// almost always a typo - surface it precisely instead of silently
		// rendering an unstyled element.
		if (xml_attribute<>* st = child->first_attribute("style"))
		{
			if (!PageManager::FindStyle(st->value()))
				LOGINFO("FoxUiEngine: <%s> on page '%s' references unknown style \"%s\"\n",
						type.c_str(), mName.c_str(), st->value());
		}

		// Record this element's rect under its id (if any) so later elements can
		// anchor to it via relative placement (below:/rightof:/...).
		if (child->first_attribute("id") && !mObjects.empty())
		{
			RenderObject* ro = dynamic_cast<RenderObject*>(mObjects.back());
			if (ro)
			{
				int rx, ry, rw, rh;
				ro->GetRenderPos(rx, ry, rw, rh);
				LayoutRect rect = { rx, ry, rw, rh };
				g_layout_ids[child->first_attribute("id")->value()] = rect;
			}
		}
	}
	return true;
}

void Page::LayoutContainer(size_t firstObject, int originX, int originY,
		int spacing, bool horizontal, int defaultItemSize,
		int crossExtent, int align, int mainExtent,
		const std::vector<int>& weights)
{
	// Pass 1 - gather the renderable children (skipping non-rendering objects
	// like <action>) with their measured main- and cross-axis sizes.
	std::vector<size_t> idx;
	std::vector<int> mainSize, crossSize;
	for (size_t i = firstObject; i < mObjects.size(); i++)
	{
		RenderObject* ro = dynamic_cast<RenderObject*>(mObjects[i]);
		if (!ro)
			continue;

		int cx, cy, cw, ch;
		ro->GetRenderPos(cx, cy, cw, ch);

		// Main-axis advance: prefer intrinsic size; measure bounded-less text
		// (as GUIButton does for its label); fall back to itemwidth/itemheight.
		int advance = horizontal ? cw : ch;
		if (advance <= 0)
		{
			GUIText* txt = dynamic_cast<GUIText*>(mObjects[i]);
			if (txt)
			{
				int tw, th;
				txt->GetCurrentBounds(tw, th);
				advance = horizontal ? tw : th;
			}
		}
		if (advance <= 0)
			advance = defaultItemSize;

		idx.push_back(i);
		mainSize.push_back(advance);
		crossSize.push_back(horizontal ? ch : cw);
	}

	// Weight distribution: give weighted children a share of the leftover
	// main-axis space. Only when a main extent is known and the weights line up
	// 1:1 with the renderable children (templates/conditions would break that).
	bool useWeights = (mainExtent > 0 && weights.size() == idx.size());
	if (useWeights)
	{
		long sumW = 0;
		int fixed = 0;
		for (size_t k = 0; k < idx.size(); k++)
		{
			if (weights[k] > 0)
				sumW += weights[k];
			else
				fixed += mainSize[k];
		}
		int totalSpacing = idx.empty() ? 0 : spacing * (int)(idx.size() - 1);
		int leftover = mainExtent - fixed - totalSpacing;
		if (leftover < 0)
			leftover = 0;
		if (sumW > 0)
		{
			for (size_t k = 0; k < idx.size(); k++)
				if (weights[k] > 0)
					mainSize[k] = (int)((long)leftover * weights[k] / sumW);
		}
	}

	// Pass 2 - place each child along the main axis, aligned on the cross axis.
	int cursor = horizontal ? originX : originY;
	for (size_t k = 0; k < idx.size(); k++)
	{
		RenderObject* ro = dynamic_cast<RenderObject*>(mObjects[idx[k]]);
		int cx, cy, cw, ch;
		ro->GetRenderPos(cx, cy, cw, ch);

		// Cross-axis alignment within crossExtent (start by default).
		int crossPos = horizontal ? originY : originX;
		if (crossExtent > 0 && align != 0)
		{
			int cs = crossSize[k];
			if (align == 1)
				crossPos += (crossExtent - cs) / 2; // center
			else if (align == 2)
				crossPos += (crossExtent - cs);     // end
		}

		int newX = horizontal ? cursor : crossPos;
		int newY = horizontal ? crossPos : cursor;
		// Weighted children get their computed main-axis size; others keep theirs.
		int newW = cw, newH = ch;
		if (useWeights && weights[k] > 0)
		{
			if (horizontal)
				newW = mainSize[k];
			else
				newH = mainSize[k];
		}
		ro->SetRenderPos(newX, newY, newW, newH);

		// Move the hit-region in lockstep (GUIButton::SetRenderPos already did
		// its own; re-reading picks up any weighted resize for those too).
		ActionObject* ao = dynamic_cast<ActionObject*>(mObjects[idx[k]]);
		if (ao)
		{
			int ax, ay, aw, ah;
			ao->GetActionPos(ax, ay, aw, ah);
			ao->SetActionPos(newX, newY, aw, ah);
		}

		cursor += mainSize[k] + spacing;
	}
}

int Page::Render(void)
{
	// Render background. A fully-transparent background (e.g. an overlay like the
	// toast) draws nothing but still costs a full-screen alpha blend - skip it.
	if (mBackground.alpha != 0) {
		gr_color(mBackground.red, mBackground.green, mBackground.blue, mBackground.alpha);
		gr_fill(0, 0, gr_fb_width(), gr_fb_height());
	}

	// Render remaining objects
	std::vector<RenderObject*>::iterator iter;
	for (iter = mRenders.begin(); iter != mRenders.end(); iter++)
	{
		if ((*iter)->Render())
			LOGERR("A render request has failed.\n");
	}
	return 0;
}

int Page::Update(void)
{
	int retCode = 0;

	std::vector<RenderObject*>::iterator iter;
	for (iter = mRenders.begin(); iter != mRenders.end(); iter++)
	{
		int ret = (*iter)->Update();
		if (ret < 0)
			LOGERR("An update request has failed.\n");
		else if (ret > retCode)
			retCode = ret;
	}

	return retCode;
}

int Page::NotifyTouch(TOUCH_STATE state, int x, int y)
{
	// By default, return 1 to ignore further touches if nobody is listening
	int ret = 1;

	// Don't try to handle a lack of handlers
	if (mActions.size() == 0)
		return ret;

	// We record mTouchStart so we can pass all the touch stream to the same handler
	if (state == TOUCH_START)
	{
		std::vector<ActionObject*>::reverse_iterator iter;
		// We work backwards, from top-most element to bottom-most element
		for (iter = mActions.rbegin(); iter != mActions.rend(); iter++)
		{
			if ((*iter)->IsInRegion(x, y))
			{
				mTouchStart = (*iter);
				ret = mTouchStart->NotifyTouch(state, x, y);
				if (ret >= 0)
					break;
				mTouchStart = NULL;
			}
		}
	}
	else if (state == TOUCH_RELEASE && mTouchStart != NULL)
	{
		ret = mTouchStart->NotifyTouch(state, x, y);
		mTouchStart = NULL;
	}
	else if ((state == TOUCH_DRAG || state == TOUCH_HOLD || state == TOUCH_REPEAT) && mTouchStart != NULL)
	{
		ret = mTouchStart->NotifyTouch(state, x, y);
	}
	return ret;
}

int Page::NotifyKey(int key, bool down)
{
	std::vector<ActionObject*>::reverse_iterator iter;

	int ret = 1;
	// We work backwards, from top-most element to bottom-most element
	for (iter = mActions.rbegin(); iter != mActions.rend(); iter++)
	{
		ret = (*iter)->NotifyKey(mime > 500 ? key + 200 : key, down);
		if (ret == 0)
			return 0;
		if (ret < 0) {
			LOGERR("An action handler has returned an error\n");
			ret = 1;
		}
	}
	return ret;
}

int Page::NotifyCharInput(int ch)
{
	std::vector<InputObject*>::reverse_iterator iter;

	// We work backwards, from top-most element to bottom-most element
	for (iter = mInputs.rbegin(); iter != mInputs.rend(); iter++)
	{
		int ret = (*iter)->NotifyCharInput(ch);
		if (ret == 0)
			return 0;
		else if (ret < 0)
			LOGERR("A char input handler has returned an error\n");
	}
	return 1;
}

int Page::SetKeyBoardFocus(int inFocus)
{
	std::vector<InputObject*>::reverse_iterator iter;

	// We work backwards, from top-most element to bottom-most element
	for (iter = mInputs.rbegin(); iter != mInputs.rend(); iter++)
	{
		int ret = (*iter)->SetInputFocus(inFocus);
		if (ret == 0)
			return 0;
		else if (ret < 0)
			LOGERR("An input focus handler has returned an error\n");
	}
	return 1;
}

void Page::SetPageFocus(int inFocus)
{
	// Render remaining objects
	std::vector<RenderObject*>::iterator iter;
	for (iter = mRenders.begin(); iter != mRenders.end(); iter++)
		(*iter)->SetPageFocus(inFocus);

	if (inFocus == 0 && mFocusedObjectIndex >= 0 && mFocusedObjectIndex < mActions.size()) {
		std::vector<ActionObject*>::iterator iter;
		for (iter = mActions.begin(); iter != mActions.end(); iter++)
			(*iter)->SetFocus(inFocus);

		mFocusedObjectIndex = -1;
	}

	return;
}

int Page::NotifyVarChange(std::string varName, std::string value)
{
	std::vector<GUIObject*>::iterator iter;
	for (iter = mObjects.begin(); iter != mObjects.end(); ++iter)
	{
		if ((*iter)->NotifyVarChange(varName, value))
			LOGERR("An action handler errored on NotifyVarChange.\n");
	}

	return 0;
}


// transient data for loading themes
struct LoadingContext
{
	ZipArchiveHandle zip; // zip to load theme from, or NULL for the stock theme
	std::set<std::string> filenames; // to detect cyclic includes
	std::string basepath; // if zip is NULL, base path to load includes from with trailing slash, otherwise empty
	std::vector<xml_document<>*> xmldocs; // all loaded xml docs
	std::vector<char*> xmlbuffers; // text buffers with xml content
	std::vector<xml_node<>*> styles; // refer to <styles> nodes inside xmldocs
	std::vector<xml_node<>*> templates; // refer to <templates> nodes inside xmldocs

	LoadingContext()
	{
		zip = NULL;
	}

	~LoadingContext()
	{
		// free all xml buffers
		for (std::vector<char*>::iterator it = xmlbuffers.begin(); it != xmlbuffers.end(); ++it)
			free(*it);
	}

};

// for FindStyle
LoadingContext* PageManager::currentLoadingContext = NULL;


PageSet::PageSet()
{
	mResources = new ResourceManager;
	mCurrentPage = nullptr;
	mCurrentOverlay = nullptr;

	set_scale_values(1, 1); // Reset any previous scaling values
}

PageSet::~PageSet()
{
	mOverlays.clear();
	for (std::vector<Page*>::iterator itr = mPages.begin(); itr != mPages.end(); ++itr)
		delete *itr;

	delete mResources;
}

int PageSet::Load(LoadingContext& ctx, const std::string& filename)
{
	bool isMain = ctx.xmlbuffers.empty(); // if we have no files yet, remember that this is the main XML file

	if (!ctx.filenames.insert(filename).second)
		// ignore already loaded files to prevent crash with cyclic includes
		return 0;

	// load XML into buffer
	char* xmlbuffer = PageManager::LoadFileToBuffer(filename, ctx.zip);
	if (!xmlbuffer)
		return -1; // error already displayed by LoadFileToBuffer
	ctx.xmlbuffers.push_back(xmlbuffer);

	// parse XML
	xml_document<>* doc = new xml_document<>();
	doc->parse<0>(xmlbuffer);
	ctx.xmldocs.push_back(doc);

	xml_node<>* root = doc->first_node("recovery");
	if (!root)
		root = doc->first_node("install");
	if (!root) {
		LOGERR("Unknown root element in %s\n", filename.c_str());
		return -1;
	}

	if (isMain) {
		int rc = LoadDetails(ctx, root);
		if (rc != 0)
			return rc;
	}

	LOGINFO("Loading resources...\n");
	xml_node<>* child = root->first_node("resources");
	if (child)
		mResources->LoadResources(child, ctx.zip, "theme");

	LOGINFO("Loading variables...\n");
	child = root->first_node("variables");
	if (child)
		LoadVariables(child);

	LOGINFO("Loading mouse cursor...\n");
	child = root->first_node("mousecursor");
	if (child)
		PageManager::LoadCursorData(child);

	LOGINFO("Loading pages...\n");
	child = root->first_node("templates");
	if (child)
		ctx.templates.push_back(child);

	child = root->first_node("styles");
	if (child)
		ctx.styles.push_back(child);

	// Load pages
	child = root->first_node("pages");
	if (child) {
		if (LoadPages(ctx, child)) {
			LOGERR("PageSet::Load returning -1\n");
			return -1;
		}
	}

	// process includes recursively
	child = root->first_node("include");
	if (child) {
		xml_node<>* include = child->first_node("xml");
		while (include != NULL) {
			xml_attribute<>* attr = include->first_attribute("name");
			if (!attr) {
				LOGERR("Skipping include/xmlfile with no name\n");
				continue;
			}

			string filename = gui_parse_text(attr->value());
			LOGINFO("Including file: %s...\n", filename.c_str());
			int rc = Load(ctx, filename);
			if (rc != 0) {
				attr = include->first_attribute("default");
				if (attr) {
					string filename = attr->value();
					int rc = Load(ctx, filename);
					if (rc != 0)
						return rc;
				} else {
					return rc;
				}
			}

			include = include->next_sibling("xml");
		}
	}

	return 0;
}

void PageSet::MakeEmergencyConsoleIfNeeded()
{
	if (mPages.empty()) {
		mCurrentPage = new Page(NULL, NULL); // fallback console page
		// TODO: since removal of non-TTF fonts, the emergency console doesn't work without a font, which might be missing too
		mPages.push_back(mCurrentPage);
	}
}

int PageSet::LoadLanguage(char* languageFile, ZipArchiveHandle package)
{
	xml_document<> lang;
	xml_node<>* parent;
	xml_node<>* child;
	std::string resource_source;
	int ret = 0;

	if (languageFile) {
		printf("parsing languageFile\n");
		lang.parse<0>(languageFile);
		printf("parsing languageFile done\n");
	} else {
		return -1;
	}

	parent = lang.first_node("language");
	if (!parent) {
		LOGERR("Unable to locate language node in language file.\n");
		lang.clear();
		return -1;
	}

	child = parent->first_node("display");
	if (child) {
		DataManager::SetValue("tw_language_display", child->value());
		resource_source = child->value();
	} else {
		LOGERR("language file does not have a display value set\n");
		DataManager::SetValue("tw_language_display", "Not Set");
		resource_source = languageFile;
	}

	child = parent->first_node("resources");
	if (child)
		mResources->LoadResources(child, package, resource_source);
	else
		ret = -1;
	DataManager::SetValue("tw_backup_name", gui_lookup("auto_generate", "(Auto Generate)"));
	lang.clear();
	return ret;
}

int PageSet::LoadDetails(LoadingContext& ctx, xml_node<>* root)
{
	xml_node<>* child = root->first_node("details");
	if (child) {
		int theme_ver = 0;
		xml_node<>* themeversion = child->first_node("themeversion");
		if (themeversion && themeversion->value()) {
			theme_ver = atoi(themeversion->value());
		} else {
			LOGINFO("No themeversion in theme.\n");
		}
		if (theme_ver != TW_THEME_VERSION) {
			LOGINFO("theme version from xml: %i, expected %i\n", theme_ver, TW_THEME_VERSION);
			if (ctx.zip) {
				gui_err("theme_ver_err=Custom theme version does not match OrangeFox version. Using stock theme.");
				return TW_THEME_VER_ERR;
			} else {
				gui_print_color("warning", "Stock theme version does not match OrangeFox version.\n");
			}
		}
		xml_node<>* resolution = child->first_node("resolution");
		if (resolution) {
			LOGINFO("Checking resolution...\n");
			xml_attribute<>* width_attr = resolution->first_attribute("width");
			xml_attribute<>* resize_attr = resolution->first_attribute("resizing");
			int height;
			if (resize_attr) {
				xml_attribute<>* height_res_attr = resolution->first_attribute("height");
				height = atoi(height_res_attr->value());
			} else {
				DataManager::GetValue("screen_original_h", height);
			}
			xml_attribute<>* noscale_attr = resolution->first_attribute("noscaling");
			if (width_attr && !noscale_attr) {
				int width = atoi(width_attr->value());
				int offx = 0, offy = 0;
#ifdef TW_ROUND_SCREEN
				xml_node<>* roundscreen = child->first_node("roundscreen");
				if (roundscreen) {
					LOGINFO("TW_ROUND_SCREEN := true, using round screen XML settings.\n");
					xml_attribute<>* offx_attr = roundscreen->first_attribute("offset_x");
					xml_attribute<>* offy_attr = roundscreen->first_attribute("offset_y");
					if (offx_attr) {
						offx = atoi(offx_attr->value());
					}
					if (offy_attr) {
						offy = atoi(offy_attr->value());
					}
				}
#endif
				if (width != 0 && height != 0) {
					std::string num;
					float scale_w, scale_h;
					//[f/d] Custom scaling for testing, wew
					//      I used file because DataManager not loaded user vars at this moment
					//      Someone may mess up recovery using this file so just remove code when OF lab disabled
#ifdef OF_ENABLE_LAB
					if (TWFunc::read_file(Fox_Home + "/scaling", num) == 0) {
							LOGERR("Custom scaling: %s\n", num.c_str());
							scale_w = ::atof(num.c_str());
							scale_h = ::atof(num.c_str());
					} else {
#endif
						scale_w = (((float)gr_fb_width() + (float)tw_w_offset) - ((float)offx * 2.0)) / (float)width;
						scale_h = (((float)gr_fb_height() + (float)tw_h_offset) - ((float)offy * 2.0)) / (float)height;
#ifdef OF_ENABLE_LAB
					}
#endif
#ifdef TW_ROUND_SCREEN
					float scale_off_w = ((float)gr_fb_width() + (float)tw_w_offset) / (float)width;
					float scale_off_h = ((float)gr_fb_height() + (float)tw_h_offset) / (float)height;
					tw_x_offset = offx * scale_off_w;
					tw_y_offset = offy * scale_off_h;
#endif
					if (scale_w != 1 || scale_h != 1) {
						LOGINFO("Scaling theme width %fx and height %fx, offsets x: %i y: %i w: %i h: %i\n",
							scale_w, scale_h, tw_x_offset, tw_y_offset, tw_w_offset, tw_h_offset);
						set_scale_values(scale_w, scale_h);
					}
				}
			} else {
				LOGINFO("XML does not contain width and height, no scaling will be applied\n");
			}
		} else {
			LOGINFO("XML contains no resolution tag, no scaling will be applied.\n");
		}
	} else {
		LOGINFO("XML contains no details tag, no scaling will be applied.\n");
	}

	return 0;
}

int PageSet::SetPage(std::string page)
{
	Page* tmp = FindPage(page);
	if (tmp)
	{
		if (mCurrentPage)   mCurrentPage->SetPageFocus(0);
		mCurrentPage = tmp;
		mCurrentPage->SetPageFocus(1);
		mCurrentPage->NotifyVarChange("", "");
		return 0;
	}
	else
	{
		LOGERR("Unable to locate page (%s)\n", page.c_str());
	}
	return -1;
}

int PageSet::SetOverlay(Page* page)
{
	// Re-entrancy guard: SetOverlay runs NotifyVarChange on the overlay's
	// objects, which can run an <action> that requests an overlay again. That
	// would recurse until the stack overflows. Block only the nested call (a
	// normal, sequential SetOverlay is unaffected).
	static bool in_set_overlay = false;
	if (in_set_overlay)
		return 0;
	struct ReentryGuard { bool& f; ~ReentryGuard() { f = false; } } rg{in_set_overlay};
	in_set_overlay = true;

	if (page) {
		if (mOverlays.size() >= 10) {
			LOGERR("Too many overlays requested, max is 10.\n");
			return -1;
		}

		std::vector<Page*>::iterator iter;
		for (iter = mOverlays.begin(); iter != mOverlays.end(); iter++) {
			if ((*iter)->GetName() == page->GetName()) {
				mOverlays.erase(iter);
				// SetOverlay() is (and should stay) the only function which
				// adds to mOverlays. Then, each page can appear at most once.
				break;
			}
		}

		mCurrentOverlay = page;
		page->SetPageFocus(1);
		page->NotifyVarChange("", "");

		if (!mOverlays.empty())
			mOverlays.back()->SetPageFocus(0);

		mOverlays.push_back(page);
	} else {
		if (!mOverlays.empty()) {
			mOverlays.back()->SetPageFocus(0);
			mOverlays.pop_back();
			if (!mOverlays.empty()) {
				mOverlays.back()->SetPageFocus(1);
				mCurrentOverlay = mOverlays.back();
			} else if (mCurrentPage) {
				mCurrentPage->SetPageFocus(1); // Just in case somehow the regular page lost focus, we'll set it again
				mCurrentOverlay = nullptr;
			}
		} else {
			mCurrentOverlay = nullptr;
		}
	}
	return 0;
}

const ResourceManager* PageSet::GetResources()
{
	return mResources;
}

Page* PageSet::FindPage(std::string name)
{
	std::vector<Page*>::iterator iter;

	for (iter = mPages.begin(); iter != mPages.end(); iter++)
	{
		if (name == (*iter)->GetName())
			return (*iter);
	}
	return NULL;
}

int PageSet::LoadVariables(xml_node<>* vars)
{
	xml_node<>* child;
	xml_attribute<> *name, *value, *persist;
	int p;

	child = vars->first_node("variable");
	while (child)
	{
		name = child->first_attribute("name");
		value = child->first_attribute("value");
		persist = child->first_attribute("persist");
		if (name && value)
		{
			if (strcmp(name->value(), "tw_x_offset") == 0) {
				tw_x_offset = atoi(value->value());
				child = child->next_sibling("variable");
				continue;
			}
			if (strcmp(name->value(), "tw_y_offset") == 0) {
				tw_y_offset = atoi(value->value());
				child = child->next_sibling("variable");
				continue;
			}
			if (strcmp(name->value(), "tw_w_offset") == 0) {
				tw_w_offset = atoi(value->value());
				child = child->next_sibling("variable");
				continue;
			}
			if (strcmp(name->value(), "tw_h_offset") == 0) {
				tw_h_offset = atoi(value->value());
				child = child->next_sibling("variable");
				continue;
			}
			p = persist ? atoi(persist->value()) : 0;
			string temp = value->value();
			string valstr = gui_parse_text(temp);

			if (valstr.find("+") != string::npos) {
				string val1str = valstr;
				val1str = val1str.substr(0, val1str.find('+'));
				string val2str = valstr;
				val2str = val2str.substr(val2str.find('+') + 1, string::npos);
				int val1 = atoi(val1str.c_str());
				int val2 = atoi(val2str.c_str());
				int val = val1 + val2;

				DataManager::SetValue(name->value(), val, p);
			} else if (valstr.find("-") != string::npos) {
				string val1str = valstr;
				val1str = val1str.substr(0, val1str.find('-'));
				string val2str = valstr;
				val2str = val2str.substr(val2str.find('-') + 1, string::npos);
				int val1 = atoi(val1str.c_str());
				int val2 = atoi(val2str.c_str());
				int val = val1 - val2;

				DataManager::SetValue(name->value(), val, p);
			} else {
				DataManager::SetValue(name->value(), valstr, p);
			}
		}

		child = child->next_sibling("variable");
	}
	return 0;
}

int PageSet::LoadPages(LoadingContext& ctx, xml_node<>* pages)
{
	xml_node<>* child;

	if (!pages)
		return -1;

	child = pages->first_node("page");
	while (child != NULL)
	{
		Page* page = new Page(child, &ctx.templates);
		if (page->GetName().empty())
		{
			LOGERR("Unable to process load page\n");
			delete page;
		}
		else
		{
			mPages.push_back(page);
		}
		child = child->next_sibling("page");
	}
	if (mPages.size() > 0)
		return 0;
	return -1;
}

int PageSet::IsCurrentPage(Page* page)
{
	return ((mCurrentPage && mCurrentPage == page) ? 1 : 0);
}

std::string PageSet::GetCurrentPage() const
{
	return mCurrentPage ? mCurrentPage->GetName() : "";
}

int PageSet::Render(void)
{
	int ret;

	ret = (mCurrentPage ? mCurrentPage->Render() : -1);
	if (ret < 0)
		return ret;

	std::vector<Page*>::iterator iter;

	for (iter = mOverlays.begin(); iter != mOverlays.end(); iter++) {
		ret = ((*iter) ? (*iter)->Render() : -1);
		if (ret < 0)
			return ret;
	}
	return ret;
}

int PageSet::Update(void)
{
	int ret;

	ret = (mCurrentPage ? mCurrentPage->Update() : -1);
	if (ret < 0 || ret > 1)
		return ret;

	std::vector<Page*>::iterator iter;

	for (iter = mOverlays.begin(); iter != mOverlays.end(); iter++) {
		ret = ((*iter) ? (*iter)->Update() : -1);
		if (ret < 0)
			return ret;
	}
	return ret;
}

int PageSet::NotifyTouch(TOUCH_STATE state, int x, int y)
{
	if (!mOverlays.empty())
		return mOverlays.back()->NotifyTouch(state, x, y);

	return (mCurrentPage ? mCurrentPage->NotifyTouch(state, x, y) : -1);
}

int PageSet::NotifyKey(int key, bool down)
{
	if (!mOverlays.empty())
		return mOverlays.back()->NotifyKey(key, down);

	return (mCurrentPage ? mCurrentPage->NotifyKey(key, down) : -1);
}

int PageSet::NotifyCharInput(int ch)
{
	if (!mOverlays.empty())
		return mOverlays.back()->NotifyCharInput(ch);

	return (mCurrentPage ? mCurrentPage->NotifyCharInput(ch) : -1);
}

int PageSet::SetKeyBoardFocus(int inFocus)
{
	if (!mOverlays.empty())
		return mOverlays.back()->SetKeyBoardFocus(inFocus);

	return (mCurrentPage ? mCurrentPage->SetKeyBoardFocus(inFocus) : -1);
}

int PageSet::NotifyVarChange(std::string varName, std::string value)
{
	std::vector<Page*>::iterator iter;

	for (iter = mOverlays.begin(); iter != mOverlays.end(); iter++)
		(*iter)->NotifyVarChange(varName, value);

	return (mCurrentPage ? mCurrentPage->NotifyVarChange(varName, value) : -1);
}

void PageSet::AddStringResource(std::string resource_source, std::string resource_name, std::string value)
{
	mResources->AddStringResource(resource_source, resource_name, value);
}

char* PageManager::LoadFileToBuffer(std::string filename, ZipArchiveHandle package) {
	size_t len;
	char* buffer = NULL;

	if (!package) {
		// We can try to load the XML directly...
		LOGINFO("PageManager::LoadFileToBuffer loading filename: '%s' directly\n", filename.c_str());
		struct stat st;
		if (stat(filename.c_str(),&st) != 0) {
			// This isn't always an error, sometimes we request files that don't exist.
			return NULL;
		}

		len = (size_t)st.st_size;

		buffer = (char*) malloc(len + 1);
		if (!buffer) {
			LOGERR("PageManager::LoadFileToBuffer failed to malloc\n");
			return NULL;
		}

		int fd = open(filename.c_str(), O_RDONLY);
		if (fd == -1) {
			LOGERR("PageManager::LoadFileToBuffer failed to open '%s' - (%s)\n", filename.c_str(), strerror(errno));
			free(buffer);
			return NULL;
		}

		if (read(fd, buffer, len) < 0) {
			LOGERR("PageManager::LoadFileToBuffer failed to read '%s' - (%s)\n", filename.c_str(), strerror(errno));
			free(buffer);
			close(fd);
			return NULL;
		}
		close(fd);
	} else {
		LOGINFO("PageManager::LoadFileToBuffer loading filename: '%s' from zip\n", filename.c_str());
		ZipEntry binary_entry;
		if (FindEntry(package, filename, &binary_entry) != 0) {
			LOGERR("Unable to locate '%s' in zip file\n", filename.c_str());
			return NULL;
		}

		// Allocate the buffer for the file
		len = binary_entry.uncompressed_length;
		buffer = (char*) malloc(len + 1);
		if (!buffer)
			return NULL;

		int32_t err =
			ExtractToMemory(package, &binary_entry, reinterpret_cast<uint8_t*>(buffer), len);
		if (err != 0) {
			LOGERR("Unable to extract '%s'\n", filename.c_str());
			free(buffer);
			return NULL;
		}
	}
	// NULL-terminate the string
	buffer[len] = 0x00;
	return buffer;
}

void PageManager::LoadLanguageListDir(string dir) {
	if (!TWFunc::Path_Exists(dir)) {
		LOGERR("LoadLanguageListDir '%s' path not found\n", dir.c_str());
		return;
	}

	DIR *d = opendir(dir.c_str());
	struct dirent *p;

	if (d == NULL) {
		LOGERR("LoadLanguageListDir error opening dir: '%s', %s\n", dir.c_str(), strerror(errno));
		return;
	}

	while ((p = readdir(d))) {
		if (!strcmp(p->d_name, ".") || !strcmp(p->d_name, "..") || strlen(p->d_name) < 5)
			continue;

		string file = p->d_name;
		if (file.substr(strlen(p->d_name) - 4) != ".xml")
			continue;
		string path = dir + p->d_name;
		string file_no_extn = file.substr(0, strlen(p->d_name) - 4);
		struct language_struct language_entry;
		language_entry.filename = file_no_extn;
		char* xmlFile = PageManager::LoadFileToBuffer(dir + p->d_name, NULL);
		if (xmlFile == NULL) {
			LOGERR("LoadLanguageListDir unable to load '%s'\n", language_entry.filename.c_str());
			continue;
		}
		xml_document<> *doc = new xml_document<>();
		doc->parse<0>(xmlFile);

		xml_node<>* parent = doc->first_node("language");
		if (!parent) {
			LOGERR("Invalid language XML file '%s'\n", language_entry.filename.c_str());
		} else {
			xml_node<>* child = parent->first_node("display");
			if (child) {
				language_entry.displayvalue = child->value();
			} else {
				LOGERR("No display value for '%s'\n", language_entry.filename.c_str());
				language_entry.displayvalue = language_entry.filename;
			}
			Language_List.push_back(language_entry);
		}
		doc->clear();
		delete doc;
		free(xmlFile);
	}
	closedir(d);
}

void PageManager::LoadLanguageList(ZipArchiveHandle package) {
	Language_List.clear();
	if (TWFunc::Path_Exists(TWRES "customlanguages"))
		TWFunc::removeDir(TWRES "customlanguages", true);
	if (package) {
		TWFunc::Recursive_Mkdir(TWRES "customlanguages");
		ExtractPackageRecursive(package, "/", TWRES "customlanguages", nullptr, nullptr);

		// package->ExtractRecursive("languages", TWRES "customlanguages/");
		LoadLanguageListDir(TWRES "customlanguages/");
	} else {
		LoadLanguageListDir(TWRES "languages/");
	}

	std::sort(Language_List.begin(), Language_List.end());
}

void PageManager::LoadLanguage(string filename) {
	string actual_filename;
	if (TWFunc::Path_Exists(TWRES "customlanguages/" + filename + ".xml"))
		actual_filename = TWRES "customlanguages/" + filename + ".xml";
	else
		actual_filename = TWRES "languages/" + filename + ".xml";
	char* xmlFile = PageManager::LoadFileToBuffer(actual_filename, NULL);
	if (xmlFile == NULL)
		LOGERR("Unable to load '%s'\n", actual_filename.c_str());
	else {
		mCurrentSet->LoadLanguage(xmlFile, NULL);
		free(xmlFile);
	}
	PartitionManager.Translate_Partition_Display_Names();
}

int PageManager::LoadPackage(std::string name, std::string package, std::string startpage)
{
	std::string mainxmlfilename = package;
	char* languageFile = NULL;
	char* baseLanguageFile = NULL;
	PageSet* pageSet = NULL;
	int ret;

	mReloadTheme = false;
	mStartPage = startpage;

	// init the loading context
	LoadingContext ctx;

	// Open the XML file
	LOGINFO("Loading package: %s (%s)\n", name.c_str(), package.c_str());
	if (package.size() > 4 && package.substr(package.size() - 4) != ".zip")
	{
		LOGINFO("Load XML directly\n");
		tw_x_offset = TW_X_OFFSET;
		tw_y_offset = TW_Y_OFFSET;
		tw_w_offset = TW_W_OFFSET;
		tw_h_offset = TW_H_OFFSET;
		if (name != "splash") {
			LoadLanguageList(NULL);
			languageFile = LoadFileToBuffer(TWRES "languages/en.xml", NULL);
		}
		ctx.basepath = TWRES;
	}
	else
	{
		LOGINFO("Loading zip theme\n");
		tw_x_offset = 0;
		tw_y_offset = 0;
		tw_w_offset = 0;
		tw_h_offset = 0;
		if (!TWFunc::Path_Exists(package)) {
			return -1;
		}

		ZipArchiveHandle Zip;
		int err = OpenArchive(package.c_str(), &Zip);

		if (err != 0)
			return -1;
		
		ctx.zip = Zip;
		mainxmlfilename = "ui.xml";
		LoadLanguageList(ctx.zip);
		languageFile = LoadFileToBuffer("languages/en.xml", ctx.zip);
		baseLanguageFile = LoadFileToBuffer(TWRES "languages/en.xml", NULL);
	}

	// Before loading, mCurrentSet must be the loading package so we can find resources
	pageSet = mCurrentSet;
	mCurrentSet = new PageSet();

	if (baseLanguageFile) {
		mCurrentSet->LoadLanguage(baseLanguageFile, NULL);
		free(baseLanguageFile);
	}

	if (languageFile) {
		mCurrentSet->LoadLanguage(languageFile, ctx.zip);
		free(languageFile);
	}

	// Load and parse the XML and all includes
	currentLoadingContext = &ctx; // required to find styles
	ret = mCurrentSet->Load(ctx, mainxmlfilename);
	currentLoadingContext = NULL;

	if (ret == 0) {
		mCurrentSet->SetPage(startpage);
		mPageSets.insert(std::pair<std::string, PageSet*>(name, mCurrentSet));
	} else {
		if (ret != TW_THEME_VER_ERR)
			LOGERR("Package %s failed to load.\n", name.c_str());
	}

	// reset to previous pageset
	mCurrentSet = pageSet;

	if (ctx.zip) {
		CloseArchive(ctx.zip);
	}
	return ret;
}

PageSet* PageManager::FindPackage(std::string name)
{
	std::map<std::string, PageSet*>::iterator iter;

	iter = mPageSets.find(name);
	if (iter != mPageSets.end())
		return (*iter).second;

	LOGERR("Unable to locate package %s\n", name.c_str());
	return NULL;
}

PageSet* PageManager::SelectPackage(std::string name)
{
	LOGINFO("Switching packages (%s)\n", name.c_str());
	PageSet* tmp;

	tmp = FindPackage(name);
	if (tmp)
	{
		mCurrentSet = tmp;
		mCurrentSet->MakeEmergencyConsoleIfNeeded();
		mCurrentSet->NotifyVarChange("", "");
	}
	else
		LOGERR("Unable to find package.\n");

	return mCurrentSet;
}

int PageManager::ReloadPackage(std::string name, std::string package)
{
	std::map<std::string, PageSet*>::iterator iter;

	mReloadTheme = false;

	iter = mPageSets.find(name);
	if (iter == mPageSets.end())
		return -1;

	if (mMouseCursor)
		mMouseCursor->ResetData(gr_fb_width(), gr_fb_height());

	PageSet* set = (*iter).second;
	mPageSets.erase(iter);

	if (LoadPackage(name, package, mStartPage) != 0)
	{
		LOGINFO("Failed to load package '%s'.\n", package.c_str());
		mPageSets.insert(std::pair<std::string, PageSet*>(name, set));
		return -1;
	}
	if (mCurrentSet == set)
		SelectPackage(name);
	delete set;
	GUIConsole::Translate_Now();
	return 0;
}

void PageManager::ReleasePackage(std::string name)
{
	std::map<std::string, PageSet*>::iterator iter;

	iter = mPageSets.find(name);
	if (iter == mPageSets.end())
		return;

	PageSet* set = (*iter).second;
	mPageSets.erase(iter);
	delete set;
	if (set == mCurrentSet)
		mCurrentSet = NULL;
	return;
}

int PageManager::RunReload() {
	int ret_val = 0;
	std::string theme_path, isPassOpen;

	//[f/d] save pass open var so we don't need to retype pass after theme changing
	DataManager::GetValue("pass_open", isPassOpen);

	if (!mReloadTheme)
		return 0;

	mReloadTheme = false;
	theme_path = DataManager::GetSettingsStoragePath();
	if (PartitionManager.Mount_By_Path(theme_path.c_str(), 1) < 0) {
		LOGERR("Unable to mount %s during gui_reload_theme function.\n", theme_path.c_str());
		ret_val = 1;
	}

	theme_path += "/theme/ui.zip";
	if (ret_val != 0 || ReloadPackage("OrangeFox", theme_path) != 0)
	{
		// Loading the custom theme failed - try loading the stock theme
		LOGINFO("Attempting to reload stock theme...\n");
		if (ReloadPackage("OrangeFox", TWRES "ui.xml"))
		{
			LOGERR("Failed to load base packages.\n");
			ret_val = 1;
		}
	}
	if (ret_val == 0) {
		if (DataManager::GetStrValue("tw_language") != "en") {
			LOGINFO("Loading language '%s'\n", DataManager::GetStrValue("tw_language").c_str());
			LoadLanguage(DataManager::GetStrValue("tw_language"));
		}
	}

	//[f/d] Load settings file
	DataManager::ReadSettingsFile();
    	gui_forceRender();
	std::string page_return;
	DataManager::GetValue("of_reload_back", page_return);
	DataManager::SetValue("pass_open", isPassOpen);
	gui_changePage(page_return);

	LOGINFO("Theme reloaded\n");

	// This makes the console re-translate
	//GUIConsole::Clear_For_Retranslation();

	return ret_val;
}

void PageManager::RequestReload() {
	DataManager::Flush();
	mReloadTheme = true;
}

void PageManager::SetStartPage(const std::string& page_name) {
	mStartPage = page_name;
}

int PageManager::ChangePage(std::string name)
{
	DataManager::SetValue("tw_operation_state", 0);
	int ret = (mCurrentSet ? mCurrentSet->SetPage(name) : -1);
	return ret;
}

std::string PageManager::GetCurrentPage()
{
	return mCurrentSet ? mCurrentSet->GetCurrentPage() : "";
}

int PageManager::ChangeOverlay(std::string name)
{
	if (name.empty())
		return mCurrentSet->SetOverlay(NULL);
	else
	{
		Page* page = mCurrentSet ? mCurrentSet->FindPage(name) : NULL;
		return mCurrentSet->SetOverlay(page);
	}
}

const ResourceManager* PageManager::GetResources()
{
	return (mCurrentSet ? mCurrentSet->GetResources() : NULL);
}

int PageManager::IsCurrentPage(Page* page)
{
	return (mCurrentSet ? mCurrentSet->IsCurrentPage(page) : 0);
}

int PageManager::Render(void)
{
	if (blankTimer.isScreenOff())
		return 0;

	int res = (mCurrentSet ? mCurrentSet->Render() : -1);
	if (mMouseCursor)
		mMouseCursor->Render();
	return res;
}

HardwareKeyboard *PageManager::GetHardwareKeyboard()
{
	if (!mHardwareKeyboard)
		mHardwareKeyboard = new HardwareKeyboard();
	return mHardwareKeyboard;
}

xml_node<>* PageManager::FindStyle(std::string name)
{
	if (!currentLoadingContext)
	{
		LOGERR("FindStyle works only while loading a theme.\n");
		return NULL;
	}

	for (std::vector<xml_node<>*>::iterator itr = currentLoadingContext->styles.begin(); itr != currentLoadingContext->styles.end(); itr++) {
		xml_node<>* node = (*itr)->first_node("style");

		while (node) {
			if (!node->first_attribute("name"))
				continue;

			if (name == node->first_attribute("name")->value())
				return node;
			node = node->next_sibling("style");
		}
	}
	return NULL;
}

MouseCursor *PageManager::GetMouseCursor()
{
	if (!mMouseCursor)
		mMouseCursor = new MouseCursor(gr_fb_width(), gr_fb_height());
	return mMouseCursor;
}

void PageManager::LoadCursorData(xml_node<>* node)
{
	if (!mMouseCursor)
		mMouseCursor = new MouseCursor(gr_fb_width(), gr_fb_height());

	mMouseCursor->LoadData(node);
}

// Deferred work marshalled onto the GUI thread via gui_run_on_main(). Drained at
// the top of PageManager::Update(); the producer side (which may be any thread)
// only touches the mutex-guarded queue, never page state.
static std::mutex g_deferred_mutex;
static std::vector<std::function<void()>> g_deferred_actions;

void gui_run_on_main(std::function<void()> fn)
{
	if (!fn)
		return;
	std::lock_guard<std::mutex> lk(g_deferred_mutex);
	g_deferred_actions.push_back(std::move(fn));
}

void PageManager::RunDeferredActions(void)
{
	std::vector<std::function<void()>> actions;
	{
		std::lock_guard<std::mutex> lk(g_deferred_mutex);
		if (g_deferred_actions.empty())
			return;
		actions.swap(g_deferred_actions);
	}
	for (auto& fn : actions)
		fn();
	// A deferred action (toast overlay, page reload) may not otherwise dirty the
	// frame; force a render so worker-driven UI updates show even while idle.
	gui_forceRender();
}

// Transient toast/snackbar: gui_toast() shows the "toast" overlay with a message
// and a frame countdown; PageManager::Update ticks it down and auto-dismisses.
static int g_toast_frames = 0;

void gui_toast(const std::string& text, int frames)
{
	// Optional bootstrap-style severity marker at the very start of the message:
	//   [success] / [warning] / [info] / [error]   (default: info).
	// It selects the toast's icon + outline colour and is stripped from the text.
	std::string msg = text;
	std::string type = "info";
	const char* colorVar = "toast_c_info";    // theme colour var name per type
	if (msg.size() > 2 && msg[0] == '[') {
		size_t close = msg.find(']');
		if (close != std::string::npos) {
			std::string tag = msg.substr(1, close - 1);
			bool known = true;
			if (tag == "success")      colorVar = "toast_c_success";
			else if (tag == "warning") colorVar = "toast_c_warning";
			else if (tag == "error")   colorVar = "toast_c_error";
			else if (tag == "info")    colorVar = "toast_c_info";
			else                       known = false;
			if (known) {
				type = tag;
				// strip "[tag]" and any single following space
				size_t start = close + 1;
				if (start < msg.size() && msg[start] == ' ')
					start++;
				msg = msg.substr(start);
			}
		}
	}

	DataManager::SetValue("toast_text", msg);
	DataManager::SetValue("toast_type", type);
	DataManager::SetValue("toast_outline", DataManager::GetStrValue(colorVar));
	g_toast_frames = frames > 0 ? frames : 1;
	PageManager::ChangeOverlay("toast");
}

int PageManager::Update(void)
{
	// Run any work marshalled from other threads first, on the GUI thread, so it
	// can safely mutate page/overlay state.
	RunDeferredActions();

	if (blankTimer.isScreenOff())
		return 0;

	// Auto-dismiss an active toast once its time runs out (done before the set
	// Update so we don't tear down the overlay mid-iteration).
	if (g_toast_frames > 0 && --g_toast_frames == 0)
		ChangeOverlay("");

	if (RunReload())
		return -2;

	int res = (mCurrentSet ? mCurrentSet->Update() : -1);

	if (mMouseCursor)
	{
		int c_res = mMouseCursor->Update();
		if (c_res > res)
			res = c_res;
	}
	return res;
}

int PageManager::NotifyTouch(TOUCH_STATE state, int x, int y)
{
	return (mCurrentSet ? mCurrentSet->NotifyTouch(state, x, y) : -1);
}

int PageManager::NotifyKey(int key, bool down)
{
	return (mCurrentSet ? mCurrentSet->NotifyKey(key, down) : -1);
}

int PageManager::NotifyCharInput(int ch)
{
	return (mCurrentSet ? mCurrentSet->NotifyCharInput(ch) : -1);
}

int PageManager::SetKeyBoardFocus(int inFocus)
{
	return (mCurrentSet ? mCurrentSet->SetKeyBoardFocus(inFocus) : -1);
}

int PageManager::NotifyVarChange(std::string varName, std::string value)
{
	return (mCurrentSet ? mCurrentSet->NotifyVarChange(varName, value) : -1);
}

void PageManager::AddStringResource(std::string resource_source, std::string resource_name, std::string value)
{
	if (mCurrentSet)
		mCurrentSet->AddStringResource(resource_source, resource_name, value);
}

void PageManager::MoveFocus(Page::Direction direction)
{
	if (mCurrentSet) mCurrentSet->MoveFocus(direction);
}

void PageManager::SelectFocusedElement(bool longPressed)
{
	if (mCurrentSet) mCurrentSet->SelectFocusedElement(longPressed);
}

void PageSet::MoveFocus(Page::Direction direction)
{
	if (mCurrentOverlay)
		mCurrentOverlay->MoveFocus(direction);
	else if (mCurrentPage)
		mCurrentPage->MoveFocus(direction);
}

void PageSet::SelectFocusedElement(bool longPressed)
{
	if (mCurrentOverlay)
		mCurrentOverlay->SelectFocusedElement(longPressed);
	else if (mCurrentPage)
		mCurrentPage->SelectFocusedElement(longPressed);
}

int Page::MoveFocusIndex(Page::Direction direction) {
	static bool keepFocusIndex = false;
	int nextIndex = mFocusedObjectIndex;
	size_t actionsSize = mActions.size();

	if (!keepFocusIndex)
		nextIndex -= static_cast<int>(direction);
	else
		keepFocusIndex = false;

	do {
		// Do not swap the checks
		if (nextIndex < 0)
			nextIndex = actionsSize - 1;

		if (nextIndex >= actionsSize)
			nextIndex = 0;

		ActionObject* focusedElement = mActions[nextIndex];
		GUIObject* obj = dynamic_cast<GUIObject*>(focusedElement);
		if (obj && !obj->isConditionTrue())
			continue;
		string name = focusedElement->GetObjectType();

		if (name == "GUIScrollList") {
			IInteractiveScrollList* scrollList = dynamic_cast<IInteractiveScrollList*>(focusedElement);
			if (scrollList && scrollList->GetItemCount() != 0) {
				if (!focusedElement->HasFocus()) {
					if (direction == Page::Direction::Down)
						scrollList->SetSelectedItem(0);
					else
						scrollList->SetSelectedItem(scrollList->GetItemCount() - 1);
					gui_forceRender();
					keepFocusIndex = true;
					return nextIndex; // Stay here
				} else if ((direction == Page::Direction::Down && scrollList->MoveSelectionDown()) || (direction == Page::Direction::Up && scrollList->MoveSelectionUp())) {
					gui_forceRender();
					keepFocusIndex = true;
					return nextIndex; // Stay here
				} else {
					// End of scrollList
					focusedElement->SetFocus(0);
					continue; // Searching new index
				}
			} else {
				continue;
			}
		} else if (name == "GUIScrollContainer") {
			// A scroll viewport navigates like a list: its focusable children
			// (embedded listboxes) are chained into one selection sequence and
			// auto-scrolled into view. See GUIScrollContainer.
			GUIScrollContainer* scroll = dynamic_cast<GUIScrollContainer*>(focusedElement);
			if (scroll && scroll->GetItemCount() != 0) {
				if (!focusedElement->HasFocus()) {
					if (direction == Page::Direction::Down)
						scroll->SetSelectedItem(0);
					else
						scroll->SetSelectedItem(scroll->GetItemCount() - 1);
					gui_forceRender();
					keepFocusIndex = true;
					return nextIndex; // Stay here
				} else if ((direction == Page::Direction::Down && scroll->MoveSelectionDown()) || (direction == Page::Direction::Up && scroll->MoveSelectionUp())) {
					gui_forceRender();
					keepFocusIndex = true;
					return nextIndex; // Stay here
				} else {
					// End of the scroll container
					focusedElement->SetFocus(0);
					continue; // Searching new index
				}
			} else {
				continue;
			}
		} else if (name == "GUIKeyboard") {
			GUIKeyboard* keyboard = dynamic_cast<GUIKeyboard*>(focusedElement);
			if (keyboard) {
				if (!focusedElement->HasFocus()) {
					if (direction == Page::Direction::Down)
						keyboard->SetSelectedItem(true);
					else
						keyboard->SetSelectedItem(false);
					gui_forceRender();
					keepFocusIndex = true;
					return nextIndex; // Stay here
				} else if ((direction == Page::Direction::Down && keyboard->MoveSelectionNext()) || (direction == Page::Direction::Up && keyboard->MoveSelectionPrevious())) {
					gui_forceRender();
					keepFocusIndex = true;
					return nextIndex; // Stay here
				} else {
					// End of keyboard
					focusedElement->SetFocus(0);
					continue; // Searching new index
				}
			} else {
				continue;
			}
		} else if (name == "GUIButton") {
			GUIButton* button = dynamic_cast<GUIButton*>(focusedElement);
			if (button && button->GetActionCount() > 0) {
				return nextIndex; // Next is that button
			} else {
				continue; // Searching new index
			}
		} else if (name == "GUICheckbox" || name == "GUISlider" || name == "GUISliderValue" || name == "GUIPatternPassword") {
			return nextIndex; // Next is that object
		}

	} while ((nextIndex -= static_cast<int>(direction)) != mFocusedObjectIndex); // We work backwards, from top-most element to bottom-most element
	return nextIndex; // Unable to find required object
}

void Page::ShiftSlider(Page::Direction direction)
{
	ActionObject* focusedElement = mActions[mFocusedObjectIndex];
	GUISlider* slider = dynamic_cast<GUISlider*>(focusedElement);
	if (slider) {
		int moveToX = slider->GetValXCurr() + static_cast<int>(direction) * (sliderEndX - sliderStartX) / 2;
		if (moveToX < sliderStartX)
			moveToX = sliderStartX;
		if (moveToX >= sliderEndX)
			moveToX = sliderEndX;
		focusedElement->NotifyTouch(TOUCH_DRAG, moveToX, sliderY);
	}
}

void Page::ShiftSliderVal(Page::Direction direction)
{
	ActionObject* focusedElement = mActions[mFocusedObjectIndex];
	GUISliderValue* slider = dynamic_cast<GUISliderValue*>(focusedElement);
	if (slider) {
		int value = slider->GetCurrentValue() + static_cast<int>(direction) * (sliderEndX - sliderStartX) / 10;
		slider->SetCurrentValue(value);
	}
}

void Page::MoveFocusInPattern(Page::Direction direction)
{
	GUIPatternPassword* patternPass = dynamic_cast<GUIPatternPassword*>(mActions[mFocusedObjectIndex]);
	if (patternPass) {
		if (direction == Page::Direction::Down)
			patternPass->MoveSelectionNext();
		else
			patternPass->MoveSelectionPrevious();
		gui_forceRender();
	}
}

void Page::MoveFocus(Page::Direction direction)
{
	if (mActions.empty()) return;

	if (mFocusSlider)
		return ShiftSlider(direction);

	if (mFocusSliderVal)
		return ShiftSliderVal(direction);

	if (mFocusPatternPassword)
		return MoveFocusInPattern(direction);

	SetFocus(MoveFocusIndex(direction));
}

void Page::SelectFocusedElement(bool longPressed) {
	if (mFocusedObjectIndex >= 0 && mFocusedObjectIndex < mActions.size()) {
		ActionObject* focusedElement = mActions[mFocusedObjectIndex];
		GUISlider* slider = dynamic_cast<GUISlider*>(focusedElement);
		GUISliderValue* sliderVal = dynamic_cast<GUISliderValue*>(focusedElement);
		GUIPatternPassword* patternPass = dynamic_cast<GUIPatternPassword*>(focusedElement);
		int centerX, centerY, actionW, actionH;

		if (slider) {
			if (!mFocusSlider) {
				mFocusSlider = true;
				slider->GetSliderPos(sliderStartX, sliderEndX, sliderY);
				slider->mFocusColor = {0, 255, 0, 255};
				focusedElement->NotifyTouch(TOUCH_START, sliderStartX, sliderY);
			} else {
				mFocusSlider = false;
				slider->mFocusColor = {255, 0, 0, 255};
				focusedElement->NotifyTouch(TOUCH_RELEASE, sliderEndX, sliderY);
			}
			gui_forceRender();
			return;
		} else if (sliderVal) {
			if (!mFocusSliderVal) {
				mFocusSliderVal = true;
				sliderVal->GetSliderPos(sliderStartX, sliderEndX, sliderY);
				sliderVal->mFocusColor = {0, 255, 0, 255};
			} else {
				mFocusSliderVal = false;
				sliderVal->mFocusColor = {255, 0, 0, 255};
			}
			gui_forceRender();
			return;
		} else if (patternPass) {
			static bool isFirstDot = true;
			if (!mFocusPatternPassword) { // We are not in the pattern; Enter
				mFocusPatternPassword = true;
				patternPass->mFocusColor = {0, 255, 0, 255};
			} else if (longPressed) { // We are already in the pattern; Exit
				mFocusPatternPassword = false;
				isFirstDot = true;
				patternPass->mFocusColor = {255, 0, 0, 255};
				focusedElement->NotifyTouch(TOUCH_RELEASE, 0, 0); // End drawing (no dots will be selected here, the coordinates will not be checked)
			} else { // We are already in the pattern; Select dot
				focusedElement->GetFocusedItemActionPos(centerX, centerY, actionW, actionH);
				centerX += actionW / 2;
				centerY += actionH / 2;
				if (isFirstDot) {
					focusedElement->NotifyTouch(TOUCH_START, centerX, centerY); // 1st dot
					isFirstDot = false;
				} else {
					focusedElement->NotifyTouch(TOUCH_DRAG, centerX, centerY); // 2nd and others dots
				}
			}
			return;
		} else if (focusedElement->GetFocusedItemActionPos(centerX, centerY, actionW, actionH)) {
			centerX += actionW / 2;
			centerY += actionH / 2;
		} else {
			focusedElement->GetActionPos(centerX, centerY, actionW, actionH);
			centerX += actionW / 2;
			centerY += actionH / 2;
		}

		GUIKeyboard* keyboard = dynamic_cast<GUIKeyboard*>(focusedElement);
		if (keyboard) {
			focusedElement->NotifyTouch(TOUCH_START, centerX, centerY);

			if (longPressed)
				focusedElement->NotifyTouch(TOUCH_HOLD, centerX, centerY);
			else
				focusedElement->NotifyTouch(TOUCH_RELEASE, centerX, centerY);

			return;
		}

		if (longPressed)
			focusedElement->NotifyTouch(TOUCH_HOLD, centerX, centerY);
		else
			focusedElement->NotifyTouch(TOUCH_RELEASE, centerX, centerY);
	}
}

void Page::SetFocus(int index)
{
	if (mFocusedObjectIndex >= 0 && mFocusedObjectIndex < mActions.size())
		mActions[mFocusedObjectIndex]->SetFocus(0);

	mFocusedObjectIndex = index;
	if (mFocusedObjectIndex >= 0 && mFocusedObjectIndex < mActions.size())
		mActions[mFocusedObjectIndex]->SetFocus(1);

	gui_forceRender();
}

extern "C" void gui_notifyVarChange(const char *name, const char* value)
{
	if (!gGuiRunning)
		return;

	PageManager::NotifyVarChange(name, value);
}
