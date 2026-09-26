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
		int iconRealX = textW + mPadding + mRenderW;

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
			int height = mDH * mBatteryPercent / 100;
			gr_fill(mDX - iconRealX, mDY+mDH-height, mDW + 1, height + 1);
		}

		if (!finalImage || !finalImage->GetResource())
			return -1;
		gr_blit(finalImage->GetResource(), 0, 0, mRenderW, mRenderH, mRenderX - iconRealX, mRenderY);


		if (!mCharge || !mCharge->GetResource())
			return 0;
		if (mBatteryCharge == 1)
			gr_blit(mCharge->GetResource(), 0, 0, mCW, mCH, mCX - iconRealX, mCY);

	}

	gr_textEx_scaleW(mRenderX - textW, mRenderY + ((mRenderH - mFontHeight) / 2) - 2,
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
