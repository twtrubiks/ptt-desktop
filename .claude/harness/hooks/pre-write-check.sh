#!/usr/bin/env bash
# PreToolUse hook (Write|Edit)：擋危險寫入。exit 2 = 阻擋，其餘放行。
set -u

INPUT=$(cat)

FILE_PATH=$(printf '%s' "$INPUT" | grep -oP '"file_path"\s*:\s*"\K[^"]+' | head -1)
[ -z "${FILE_PATH:-}" ] && exit 0

deny() {
  echo "pre-write-check: $1" >&2
  exit 2
}

case "$FILE_PATH" in
  *.env | *.env.*)
    case "$FILE_PATH" in
      *.example | *.sample | *.template) ;;
      *) deny "禁止寫入 .env（範本請用 .env.example）: $FILE_PATH" ;;
    esac
    ;;
  */node_modules/*) deny "禁止寫入 node_modules: $FILE_PATH" ;;
  */.git/*) deny "禁止寫入 .git: $FILE_PATH" ;;
  */src-tauri/target/* | */dist/*) deny "禁止寫入建置產物目錄: $FILE_PATH" ;;
  "$HOME/.config/ptt-desktop/"*) deny "禁止寫入真實設定目錄（含帳密）: $FILE_PATH" ;;
esac

# 內容掃描（content / new_string）：高信度 secret pattern
CONTENT=$(printf '%s' "$INPUT" | grep -oP '"(content|new_string)"\s*:\s*"\K(\\.|[^"\\])*' | head -c 200000)
if [ -n "${CONTENT:-}" ]; then
  if printf '%s' "$CONTENT" | grep -qE 'AKIA[A-Z0-9]{16}'; then
    deny "內容疑似含 AWS access key"
  fi
  if printf '%s' "$CONTENT" | grep -qE 'sk-[a-zA-Z0-9]{20,}'; then
    deny "內容疑似含 API key（sk-...）"
  fi
  # 密碼欄位掃描：tests/ 與 *.example.* 允許假資料，其餘擋下
  case "$FILE_PATH" in
    */tests/* | *.example.* | *.test.*) ;;
    *)
      if printf '%s' "$CONTENT" | grep -qiE '\\?"password\\?"\s*:\s*\\?"[^"\\]{4,}'; then
        deny "內容疑似含密碼欄位實值（範例請用空字串或占位字串）: $FILE_PATH"
      fi
      ;;
  esac
fi

exit 0
