/*
 * Copyright (C) 2017 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
#ifndef __EXYNOS_SOUNDTRIGGERCONF_H__
#define __EXYNOS_SOUNDTRIGGERCONF_H__

/********************************************************************/
/** ALSA Framework Sound Card & Sound Device Information            */
/**                                                                 */
/** You can find Sound Device Name from /dev/snd.                   */
/** Sound Device Name consist of Card Number & Device Number.       */
/**                                                                 */
/********************************************************************/

/* Sound Card and Mixer card Numbers based on Target Device */
/* You have to match this number with real kernel information */
#define VTS_SOUND_CARD          0
#define VTS_MIXER_CARD          0

/* PCM Nodes number for seamless and normal recording*/
#define VTS_TRICAP_DEVICE_NODE         13
#define VTS_RECORD_DEVICE_NODE         14

/* sysfs file paths for loading model binaries into VTS kernel driver */
#define VTS_HOTWORD_MODEL           "/sys/devices/platform/13810000.vts/vts_google_model"
#define VTS_SVOICE_MODEL            "/sys/devices/platform/13810000.vts/vts_svoice_model"

#define AUDIO_PRIMARY_HAL_LIBRARY_PATH "/vendor/lib/libaudioproxy.so"

// VTS Capture(Input) PCM Configuration
#define DEFAULT_VTS_CHANNELS          1       // Mono
#define DEFAULT_VTS_SAMPLING_RATE     16000   // 16KHz

#define PRIMARY_VTS_PERIOD_SIZE           160     // 160 frames, 10ms in case of 16KHz Stream
#define PRIMARY_VTS_PERIOD_COUNT          1024       // Buffer count => Total 327680 Bytes = 160 * 1(Mono) * 2(16bit PCM) * 1024(Buffer count)

/* VTS mixer controls */
#define VTS_ACTIVE_KEYPHRASE_CTL_NAME "VTS Active Keyphrase"    /* default: 0-"SVOICE", 1-"GOOGLE", 2-"SENSORY"  */
#define VTS_EXECUTION_MODE_CTL_NAME "VTS Execution Mode"        /* 0-"OFF-MODE", 1-"VOICE-TRIG-MODE", 2-"SOUND-DECTECT-MODE", 3-"VT-ALWAYS-MODE" 4-"GOOGLE-TRI-MODE */
#define VTS_VOICERECOGNIZE_START_CTL_NAME "VTS VoiceRecognize Start" /* 0-"Off", 1-On" */
#define VTS_VOICETRIGGER_VALUE_CTL_NAME "VTS VoiceTrigger Value"  /* 0 ~ 2000 ms*/

#define MAIN_MIC_CONTROL_COUNT          8
#define HEADSET_MIC_CONTROL_COUNT       8
#define MODEL_RECOGNIZE_CONTROL_COUNT   4

/* MIC Mixer controls for VTS */
char *main_mic_ctlname[] = {
    "VTS DMIC SEL",
    "VTS DMIC IF RCH EN",
    "VTS DMIC IF LCH EN",
    "VTS SYS SEL",
    "VTS HPF EN",
    "VTS HPF SEL",
    "DMIC1 Switch",
    "VTS Virtual Output Mux",
};

char *headset_mic_ctlname[] = {
    "AUXPDM1 Rate",
    "AUXPDM1 Input",
    "AUXPDM1 Output Switch",
    "AUXPDM Switch",
    "VTS DMIC SEL",
    "VTS DMIC IF RCH EN",
    "VTS DMIC IF LCH EN",
    "VTS SYS SEL",
    "VTS HPF EN",
    "VTS HPF SEL",
};

/* MIC Mixer control values */
/* FIXME : Double check this values */
int main_mic_ctlvalue[] = {
    0,  //"VTS DMIC SEL",
    1,  //"VTS DMIC IF RCH EN",
    1,  //"VTS DMIC IF LCH EN",
    1,  //"VTS SYS SEL",
    1,  //"VTS HPF EN",
    1,  //"VTS HPF SEL",
    1,  //"DMIC1 Switch",
    1,  //"VTS Virtual Output Mux",
};

int headset_mic_ctlvalue[] = {
    3,  //"AUXPDM1 Rate",
    0,  //"AUXPDM1 Input",
    1,  //"AUXPDM1 Output Switch",
    1,  //"AUXPDM Switch",
    1,  //"VTS DMIC SEL",
    1,  //"VTS DMIC IF RCH EN",
    1,  //"VTS DMIC IF LCH EN",
    1,  //"VTS SYS SEL",
    1,  //"VTS HPF EN",
    1,  //"VTS HPF SEL",
};
#endif  // __EXYNOS_SOUNDTRIGGERCONF_H__
