#!/bin/sh

# 1. Определение размера страницы памяти (4K или 16K)
PAGE_KB=$(grep -m 1 "^KernelPageSize:" /proc/self/smaps 2>/dev/null | awk '{print $2}')

if [ -z "$PAGE_KB" ]; then
    PAGE_BYTES=$(getconf PAGESIZE 2>/dev/null || getconf PAGE_SIZE 2>/dev/null)
    if [ -n "$PAGE_BYTES" ]; then
        PAGE_KB=$((PAGE_BYTES / 1024))
    fi
fi

if [ "$PAGE_KB" = "16" ]; then
    PAGE_SIZE="16k"
elif [ "$PAGE_KB" = "4" ]; then
    PAGE_SIZE="4k"
else
    PAGE_SIZE="${PAGE_KB:-unknown}k"
fi

# 2. Определение реальной версии ядра (обход спуфа)
FULL_VER=""

if [ -f /proc/config.gz ]; then
    HEADER=$( (zcat /proc/config.gz 2>/dev/null || gzip -dc /proc/config.gz 2>/dev/null) | head -n 4 | grep "Kernel Configuration" )
    FULL_VER=$(echo "$HEADER" | awk '{print $(NF-2)}')
fi

if [ -z "$FULL_VER" ] && [ -f /proc/version ]; then
    FULL_VER=$(awk '{print $3}' /proc/version | cut -d'-' -f1)
fi

if [ -z "$FULL_VER" ]; then
    FULL_VER=$(uname -r | cut -d'-' -f1)
fi

MAJOR_VER=$(echo "$FULL_VER" | cut -d'.' -f1,2)

# 3. Определение версии Android (Android release ядра)
ANDROID_VER=""

# Способ А: Из CONFIG_LOCALVERSION в конфиге сборки ядра
if [ -f /proc/config.gz ]; then
    LOCAL_VER=$( (zcat /proc/config.gz 2>/dev/null || gzip -dc /proc/config.gz 2>/dev/null) | grep "^CONFIG_LOCALVERSION=" )
    ANDROID_VER=$(echo "$LOCAL_VER" | grep -oE 'android[0-9]+')
fi

# Способ Б: Из vermagic / uname -r
if [ -z "$ANDROID_VER" ]; then
    ANDROID_VER=$(uname -r | grep -oE 'android[0-9]+')
fi

# Способ В: Из полного баннера /proc/version
if [ -z "$ANDROID_VER" ] && [ -f /proc/version ]; then
    ANDROID_VER=$(cat /proc/version | grep -oE 'android[0-9]+')
fi

# Способ Г: Fallback через getprop (если ядро кастомное и стерло суффикс)
if [ -z "$ANDROID_VER" ]; then
    PROP_REL=$(getprop ro.build.version.release 2>/dev/null)
    if [ -n "$PROP_REL" ]; then
        ANDROID_VER="android$PROP_REL"
    else
        ANDROID_VER="unknown"
    fi
fi

# Вывод результатов
echo "--------------------------------"
echo "Полная версия   : $FULL_VER"
echo "Мажорная ветка  : $MAJOR_VER"
echo "Версия Android  : $ANDROID_VER"
echo "Размер страницы : $PAGE_SIZE ($PAGE_KB KB)"
echo "Имя для модуля  : <name>_${ANDROID_VER}-${MAJOR_VER}_${PAGE_SIZE}.ko"
echo "--------------------------------"