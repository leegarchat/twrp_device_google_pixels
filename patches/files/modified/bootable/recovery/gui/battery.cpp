/*
        Copyright 2012 to 2016 bigbiff/Dees_Troy TeamWin
        This file is part of TWRP/TeamWin Recovery Project.

	Copyright (C) 2018-2023 OrangeFox Recovery Project
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

// battery.cpp - GUIBattery object by fordownloads@orangefox team

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

#include <string>

extern "C" {
#include "../twcommon.h"
}
#include "minuitwrp/minui.h"

#include "rapidxml.hpp"
#include "objects.hpp"

GUIBattery::GUIBattery(xml_node<>* node)
	: GUIObject(node)
{
	#ifdef TW_NO_BATT_PERCENT
		return;
	#endif

	mStateMode = false;
	mFont = NULL;
	mFontHeight = 0;
	xml_node <> *child;
	mImg100 = mImg75 = mImg50 = mImg25 = mImg15 = mImgc15 = mImg5 = mImg = mLowImg = NULL;

	if (!node)
		return;

	child = FindNode(node, "dynamic"); // Classic vertical battery; changes smoothly
	if (child) {
		mImg = LoadAttrImage(child, "img"); //default empty image
		mLowImg = LoadAttrImage(child, "imgLow"); // empty image for <15%
		mDX = LoadAttrIntScaleX(child, "dx", 0); //dynamic part placement
		mDY = LoadAttrIntScaleY(child, "dy", 0);
		mDW = LoadAttrIntScaleX(child, "dw", 32);
		mDH = LoadAttrIntScaleY(child, "dh", 18);
	} else {
		child = FindNode(node, "states"); // Battery based on images
		if (child) {
			mImg100 = LoadAttrImage(child, "100");
			mImg75  = LoadAttrImage(child, "75");
			mImg50  = LoadAttrImage(child, "50");
			mImg25  = LoadAttrImage(child, "25");
			mImg15  = LoadAttrImage(child, "15");
			mImgc15  = LoadAttrImage(child, "c15");
			mImg5   = LoadAttrImage(child, "5");
			mStateMode = true;
		} else {
			LOGERR("Battery object not loaded!\n");
			return;
		}
	}
	child = FindNode(node, "charging"); // charging icon
	if (child) {
		mCharge = LoadAttrImage(child, "img");
		mCX = LoadAttrIntScaleX(child, "x", 0); //placement
		mCY = LoadAttrIntScaleY(child, "y", 0);
		if (mCharge && mCharge->GetResource())
		{
			mCW = mCharge->GetWidth();
			mCH = mCharge->GetHeight();
		}
	}

	mFont = LoadAttrFont(FindNode(node, "font"), "resource");
	if (!mFont || !mFont->GetResource())
		return;

	mPadding = LoadAttrIntScaleX(FindNode(node, "font"), "padding", 0); //text padding
	mColor = LoadAttrColor(FindNode(node, "font"), "color", COLOR(0,0,0,255));
	mColorLow = LoadAttrColor(FindNode(node, "font"), "colorLow", COLOR(128,0,0,255));

	// Load the placement
	LoadPlacement(FindNode(node, "placement"), &mRenderX, &mRenderY, &mRenderW, &mRenderH, &mPlacement);
	SetPlacement(TOP_LEFT);
	
	mDX += mRenderX;
	mDY += mRenderY;
	mCX += mRenderX;
	mCY += mRenderY;
	mFontHeight = mFont->GetHeight();
}

int GUIBattery::Render(void)
{
	if (!isConditionTrue())
		return 0;

	void* fontResource = NULL;
	if (mFont)
		fontResource = mFont->GetResource();
	else
		return -1;

	std::string mBatteryPercentStr;
	int mBatteryPercent = DataManager::GetIntValue("tw_battery");
	int mBatteryCharge = DataManager::GetIntValue("charging_now");
	int mBatteryIcon = DataManager::GetIntValue("enable_battery");
	ImageResource* finalImage;

	if (mBatteryPercent > 15 || mBatteryCharge == 1)
		gr_color(mColor.red, mColor.green, mColor.blue, mColor.alpha);
	else
		gr_color(mColorLow.red, mColorLow.green, mColorLow.blue, mColorLow.alpha);

	mBatteryPercentStr = mBatteryIcon == 0 ? DataManager::GetStrValue("tw_battery_charge") :
											 DataManager::GetStrValue("tw_battery") + "%" ;
	
	int textW = twrpTruetype::gr_ttf_measureEx(mBatteryPercentStr.c_str(), fontResource);
	
	if (mBatteryIcon == 1) {
		if (mStateMode) {
				if (mBatteryPercent > 75) finalImage = mImg100;
			else if (mBatteryPercent > 50) finalImage = mImg75 ;
			else if (mBatteryPercent > 25) finalImage = mImg50 ;
			else if (mBatteryPercent > 15) finalImage = mImg25 ;
			else if (mBatteryPercent <= 15 && mBatteryCharge == 1) finalImage = mImgc15;
			else if (mBatteryPercent > 5)  finalImage = mImg15 ;
			else 				           finalImage = mImg5  ;
		} else {
			finalImage = (mBatteryPercent > 15 || mBatteryCharge == 1) ? mImg : mLowImg;
		}

		if (!finalImage || !finalImage->GetResource())
			return -1;

		// Fox: anchor the icon to its real raster size, not the layout
		// slot. The slot (mRenderW/H via ScaleX/ScaleY) and the image
		// (min-scale, aspect kept) diverge on non-1080 panels, so a
		// slot-based origin pushed the glyph off the padding/text math.
		// The icon right edge lands exactly mPadding past the widest
		// percent string on any panel and any style, with or without
		// wide-theme variants. When raster == slot (1080 panels) every
		// expression below reduces to the stock one — zero behavior
		// change there.
		int iw = finalImage->GetWidth();
		int ih = finalImage->GetHeight();
		if (iw <= 0 || ih <= 0) {
			iw = mRenderW;
			ih = mRenderH;
		}
		// Fox: the icon lives in its own fixed layout, independent of
		// the percent text width. Stock anchored the icon at
		// textW + padding, so every digit change (100 -> 99 -> 9)
		// dragged the icon sideways and the block shimmied. Anchor to
		// the widest possible string instead: the icon never moves,
		// the text stays right-aligned at mRenderX on its own.
		int maxTextW = twrpTruetype::gr_ttf_measureEx("100%", fontResource);
		if (maxTextW < textW)
			maxTextW = textW;
		int iconRealX = maxTextW + mPadding + iw;
		int ix = mRenderX - iconRealX;
		int iy = (mRenderH > ih) ? mRenderY + (mRenderH - ih) / 2 : mRenderY;

		if (!mStateMode) {
			// Slot-based fill rect mapped onto the drawn image.
			int fx = ix + (mRenderW > 0 ? (mDX - mRenderX) * iw / mRenderW : mDX - mRenderX);
			int fw = (mRenderW > 0 ? mDW * iw / mRenderW : mDW);
			int fh = (mRenderH > 0 ? mDH * ih / mRenderH : mDH);
			int fy = (mRenderH > 0 ? (mDY - mRenderY) * ih / mRenderH : mDY - mRenderY);
			int height = fh * mBatteryPercent / 100;
			gr_fill(fx, iy + fy + fh - height, fw + 1, height + 1);
		}

		gr_blit(finalImage->GetResource(), 0, 0, iw, ih, ix, iy);


		if (!mCharge || !mCharge->GetResource())
			return 0;
		if (mBatteryCharge == 1) {
			int bx = ix + (mRenderW > 0 ? (mCX - mRenderX) * iw / mRenderW : mCX - mRenderX);
			int by = iy + (mRenderH > 0 ? (mCY - mRenderY) * ih / mRenderH : mCY - mRenderY);
			gr_blit(mCharge->GetResource(), 0, 0, mCW, mCH, bx, by);
		}

	}

	// Fox: the percent text lives on its own style-independent row
	// (battery_text_y theme var — the same row the clock draws on),
	// never derived from the icon slot: switching icon styles must
	// not move the text. A theme without the var (foreign themes)
	// falls back to the stock slot-centered formula.
	int textY = DataManager::GetIntValue("battery_text_y");
	if (textY <= 0)
		textY = mRenderY + ((mRenderH - mFontHeight) / 2) - 2;
	gr_textEx_scaleW(mRenderX - textW, textY,
			  mBatteryPercentStr.c_str(), fontResource, 0, TOP_LEFT, false);

	return 0;
}

int GUIBattery::Update(void)
{
	if (!isConditionTrue())
		return 0;

	const uint_fast8_t threshold = 16;
	static uint_fast8_t updateCounter = threshold;

	// update the battery info on the status bar
	if (updateCounter)
		updateCounter--;
	else {
		updateCounter = threshold;
		return 2;
	}

	return 0;
}
