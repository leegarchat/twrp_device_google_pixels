/*
        Copyright 2012 bigbiff/Dees_Troy TeamWin
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

#include <string>
#include <cstdlib>
#include <pthread.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>
#include "pages.hpp"
#include "blanktimer.hpp"
#include "../data.hpp"
extern "C" {
#include "../twcommon.h"
}
#include "minuitwrp/minui.h"
#include "../twrp-functions.hpp"
#include "../variables.h"

blanktimer::blanktimer(void) {
	pthread_mutex_init(&mutex, NULL);
	setTime(0); // no timeout
	state = kOn;
	orig_brightness = getBrightness();
}

// Anything at or below the dim level ("5") is not a valid "screen on"
// brightness -- dim writes "5", off writes "0", and the brightness slider
// can never persist such a low value. If the captured value is unusable
// (empty, or a dim/off leftover), fall back to the configured brightness,
// then to the panel max. This guarantees unblank always restores a
// visible level instead of leaving the panel black.
static std::string usable_restore_brightness(const std::string& captured) {
	if (!captured.empty() && atoi(captured.c_str()) > 5)
		return captured;
	std::string cfg = DataManager::GetStrValue("tw_brightness");
	if (!cfg.empty() && atoi(cfg.c_str()) > 5)
		return cfg;
	return DataManager::GetStrValue("tw_brightness_max");
}

bool blanktimer::isScreenOff() {
	return state >= kOff;
}

void blanktimer::setTime(int newtime) {
	pthread_mutex_lock(&mutex);
	sleepTimer = newtime;
	pthread_mutex_unlock(&mutex);
}

void blanktimer::setTimer(void) {
	clock_gettime(CLOCK_MONOTONIC, &btimer);
}

void blanktimer::checkForTimeout() {
#ifndef TW_NO_SCREEN_TIMEOUT
	pthread_mutex_lock(&mutex);
	timespec curTime, diff;
	clock_gettime(CLOCK_MONOTONIC, &curTime);
	diff = TWFunc::timespec_diff(btimer, curTime);
	if (sleepTimer > 2 && diff.tv_sec > (sleepTimer - 2) && state == kOn) {
		orig_brightness = getBrightness();
		state = kDim;
		TWFunc::Set_Brightness("5");
	}
	if (sleepTimer && diff.tv_sec > sleepTimer && state < kOff) {
		state = kOff;
		TWFunc::Set_Brightness("0");
		TWFunc::check_and_run_script("/system/bin/postscreenblank.sh", "blank");
		PageManager::ChangeOverlay("lock");
	}
#ifndef TW_NO_SCREEN_BLANK
	if (state == kOff) {
		gr_fb_blank(true);
		state = kBlanked;
	}
#endif
	pthread_mutex_unlock(&mutex);
#endif
}

string blanktimer::getBrightness(void) {
	string result;

	if (DataManager::GetIntValue("tw_has_brightnesss_file")) {
		DataManager::GetValue("tw_brightness", result);
		if (result.empty())
			result = "255";
	}
	return result;
}

void blanktimer::resetTimerAndUnblank(void) {
#ifndef TW_NO_SCREEN_TIMEOUT
	pthread_mutex_lock(&mutex);
	setTimer();
	switch (state) {
		case kBlanked:
#ifndef TW_NO_SCREEN_BLANK
			gr_fb_blank(false);
#endif
			// TODO: this is asymmetric with postscreenblank.sh - shouldn't it be under the next case label?
			TWFunc::check_and_run_script("/system/bin/postscreenunblank.sh", "unblank");
			// No break here, we want to keep going
		case kOff:
			gui_forceRender();
			// No break here, we want to keep going
		case kDim:
			orig_brightness = usable_restore_brightness(orig_brightness);
			LOGINFO("blanktimer: unblank, restoring brightness %s\n", orig_brightness.c_str());
			if (!orig_brightness.empty())
				TWFunc::Set_Brightness(orig_brightness);
			state = kOn;
		case kOn:
			break;
	}
	pthread_mutex_unlock(&mutex);
#endif
}

void blanktimer::blank(void) {
/*  1) No need for timer handling since checkForTimeout() verifies
 *     state of screen before performing screen-off
 *  2) Assume screen-off causes issues for devices that set
 *     TW_NO_SCREEN_TIMEOUT and do not blank screen here either
 */
#ifndef TW_NO_SCREEN_TIMEOUT
	pthread_mutex_lock(&mutex);
	if (state == kOn) {
		orig_brightness = getBrightness();
		state = kOff;
		TWFunc::Set_Brightness("0");
		TWFunc::check_and_run_script("/system/bin/postscreenblank.sh", "blank");
	}
#ifndef TW_NO_SCREEN_BLANK
	if (state == kOff) {
		gr_fb_blank(true);
		state = kBlanked;
	}
#endif
	pthread_mutex_unlock(&mutex);
#endif
}

void blanktimer::toggleBlank(void) {
	if (state == kOn) {
		blank();
		PageManager::ChangeOverlay("lock");
	} else {
		resetTimerAndUnblank();
	}
}
