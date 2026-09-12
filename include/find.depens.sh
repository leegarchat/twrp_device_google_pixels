#!/bin/bash

TARGET="$1"
BASE_DIR=$(pwd)

if [ -z "$TARGET" ]; then
    echo "Использование: $0 <путь_к_бинарнику>"
    exit 1
fi

if ! command -v readelf &> /dev/null; then
    echo "Ошибка: readelf не найден."
    exit 1
fi

declare -A VISITED
declare -A FOUND_PATHS
declare -A MISSING_DEPS

analyze() {
    local file_path="$1"
    local indent="$2"

    if [[ ${VISITED[$file_path]} ]]; then
        return
    fi
    VISITED[$file_path]=1

    # Читаем зависимости
    local deps=$(readelf -d "$file_path" 2>/dev/null | grep "(NEEDED)" | sed -r 's/.*\[(.*)\].*/\1/')

    for dep in $deps; do
        local found_path=$(find "$BASE_DIR" -name "$dep" -print -quit)

        if [ -n "$found_path" ]; then
            local rel_path=${found_path#$BASE_DIR/}
            echo -e "${indent}├── \e[32m$dep\e[0m"
            
            # Сохраняем полный относительный путь
            FOUND_PATHS["$dep"]="$rel_path"
            
            analyze "$found_path" "│   $indent"
        else
            echo -e "${indent}├── \e[31m$dep\e[0m (NOT FOUND)"
            MISSING_DEPS["$dep"]=1
        fi
    done
}

if [ ! -f "$TARGET" ]; then
    echo "Файл $TARGET не найден."
    exit 1
fi

echo -e "\e[1mСтруктура дерева:\e[0m"
echo "$TARGET"
analyze "$TARGET" ""

echo -e "\n================================================"
echo -e "\e[1mИТОГОВЫЙ СПИСОК ПУТЕЙ:\e[0m"
echo "================================================"
for lib in $(echo "${!FOUND_PATHS[@]}" | tr ' ' '\n' | sort); do
    echo "${FOUND_PATHS[$lib]}"
done

echo -e "\n================================================"
echo -e "\e[1mИТОГОВЫЙ СПИСОК ИМЕН (BASENAME):\e[0m"
echo "================================================"
for lib in $(echo "${!FOUND_PATHS[@]}" | tr ' ' '\n' | sort); do
    echo "$lib"
done

if [ ${#MISSING_DEPS[@]} -gt 0 ]; then
    echo -e "\n\e[1;31mНЕ НАЙДЕНО:\e[0m"
    echo "${!MISSING_DEPS[@]}" | tr ' ' '\n' | sort
fi