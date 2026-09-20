#!/bin/bash

SHOW_PERMS=0
RD1=""
RD2=""

while [[ "$#" -gt 0 ]]; do
    case $1 in
        --perms|-p) SHOW_PERMS=1; shift ;;
        *)
            if [ -z "$RD1" ]; then RD1="$1"
            elif [ -z "$RD2" ]; then RD2="$1"
            else echo "Неизвестный аргумент: $1"; exit 1; fi
            shift
            ;;
    esac
done

if [ -z "$RD1" ] || [ -z "$RD2" ]; then
    echo "Использование: $0 [--perms] <ramdisk1.cpio> <ramdisk2.cpio>"
    exit 1
fi

if [ ! -f "$RD1" ] || [ ! -f "$RD2" ]; then
    echo "Ошибка: Один или оба файла не найдены!"
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Автопоиск magiskboot (даже если он с припиской версии)
MB_BIN=""
for name in magiskboot magiskboot_29 magiskboot_28; do
    if command -v "$name" >/dev/null 2>&1; then
        MB_BIN="$name"
        break
    fi
done

dump_archive() {
    local file="$1"
    local out="$2"
    local raw_cpio="$TMP/temp_$$.cpio"
    local cpio_src="$file"

    # Распаковка (сначала пробуем magiskboot, потом нативные lz4/gzip)
    if [ -n "$MB_BIN" ] && "$MB_BIN" decompress "$file" "$raw_cpio" >/dev/null 2>&1; then
        cpio_src="$raw_cpio"
    elif lz4 -dc "$file" > "$raw_cpio" 2>/dev/null; then
        cpio_src="$raw_cpio"
    elif gzip -dc "$file" > "$raw_cpio" 2>/dev/null; then
        cpio_src="$raw_cpio"
    fi

    # Шаг 1: Получаем пути
    cat "$cpio_src" | cpio -it 2>/dev/null > "$TMP/paths.txt"
    
    # Шаг 2: Получаем полную информацию (ls -l формат)
    cat "$cpio_src" | cpio -itv 2>/dev/null > "$TMP/raw.txt"
    
    # Шаг 3: Склеиваем надежно через AWK (без утилиты paste)
    # Вытягиваем $1 (права), $3 (UID), $4 (GID)
    awk 'NR==FNR { path[NR]=$0; next } 
    {
        print path[FNR] "|" $1 " (" $3 ":" $4 ")"
    }' "$TMP/paths.txt" "$TMP/raw.txt" | sort > "$out"

    rm -f "$raw_cpio" "$TMP/paths.txt" "$TMP/raw.txt"
    
    if [ ! -s "$out" ]; then
        echo "[!] ПРЕДУПРЕЖДЕНИЕ: Не удалось прочитать файлы из $(basename "$file")"
    fi
}

echo "[*] Анализ архива 1: $(basename "$RD1")"
dump_archive "$RD1" "$TMP/parsed1.txt"

echo "[*] Анализ архива 2: $(basename "$RD2")"
dump_archive "$RD2" "$TMP/parsed2.txt"

# --- Магия сравнения на чистом AWK ---
awk -F'|' -v show_perms="$SHOW_PERMS" -v diff_out="$TMP/diff.txt" \
    -v only1_out="$TMP/only1.txt" -v only2_out="$TMP/only2.txt" '
NR == FNR {
    f1_perms[$1] = $2
    next
}
{
    f2_path = $1
    f2_perm = $2
    
    if (f2_path in f1_perms) {
        if (show_perms == 1 && f1_perms[f2_path] != f2_perm) {
            print f2_path >> diff_out
            print "   RD1: " f1_perms[f2_path] >> diff_out
            print "   RD2: " f2_perm >> diff_out
            print "" >> diff_out
        }
        delete f1_perms[f2_path]
    } else {
        print f2_path >> only2_out
    }
}
END {
    for (path in f1_perms) {
        print path >> only1_out
    }
}' "$TMP/parsed1.txt" "$TMP/parsed2.txt"

# Вывод результатов
echo ""
echo "========================================="
echo "[-] Файлы ТОЛЬКО в: $(basename "$RD1")"
echo "========================================="
if [ -f "$TMP/only1.txt" ] && [ -s "$TMP/only1.txt" ]; then
    sort "$TMP/only1.txt" | sed 's/^/  - /'
else
    echo "  (нет отличий)"
fi

echo ""
echo "========================================="
echo "[+] Файлы ТОЛЬКО в: $(basename "$RD2")"
echo "========================================="
if [ -f "$TMP/only2.txt" ] && [ -s "$TMP/only2.txt" ]; then
    sort "$TMP/only2.txt" | sed 's/^/  + /'
else
    echo "  (нет отличий)"
fi

if [ "$SHOW_PERMS" -eq 1 ]; then
    echo ""
    echo "========================================="
    echo "[!] Общие файлы с РАЗНЫМИ правами"
    echo "========================================="
    if [ -f "$TMP/diff.txt" ] && [ -s "$TMP/diff.txt" ]; then
        cat "$TMP/diff.txt"
    else
        echo "  (права идентичны)"
    fi
fi