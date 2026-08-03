#!/usr/bin/env bash
# PreToolUse hook (Write|Edit)：擋危險寫入。
# exit 2 = 阻擋；exit 1 = 警告但放行（transcript 會顯示 stderr 首行）；exit 0 = 放行。
# 注意：exit 0 的 stderr 會被忽略，所以「解析失敗」必須用 exit 1 才看得見。
set -u

INPUT=$(cat)

deny() {
  echo "pre-write-check: $1" >&2
  exit 2
}

# 解析失敗一律放行——這是輔助防線，fail-closed 會擋掉所有寫入更糟；
# 但要用 exit 1 讓人知道這次沒檢查。
warn() {
  echo "pre-write-check: $1" >&2
  exit 1
}

# 優先用 jq 解析（正確處理跳脫引號與 unicode escape），無 jq 才退回 grep -P。
if command -v jq >/dev/null 2>&1; then
  FILE_PATH=$(printf '%s' "$INPUT" | jq -r '.tool_input.file_path // empty' 2>/dev/null)
  CONTENT=$(printf '%s' "$INPUT" |
    jq -r '[.tool_input.content, .tool_input.new_string]
           | map(select(type == "string")) | join("\n")' 2>/dev/null |
    head -c 200000)
else
  FILE_PATH=$(printf '%s' "$INPUT" | grep -oP '"file_path"\s*:\s*"\K[^"]+' 2>/dev/null | head -1)
  CONTENT=$(printf '%s' "$INPUT" |
    grep -oP '"(content|new_string)"\s*:\s*"\K(\\.|[^"\\])*' 2>/dev/null | head -c 200000)
  # [^"]+ 會停在跳脫引號 \" 之前，被截斷的路徑必定以反斜線結尾
  case "$FILE_PATH" in
    *\\) warn "路徑含跳脫引號、grep 後備解析已截斷，本次未檢查: $FILE_PATH" ;;
  esac
fi

# 空值代表 grep 不支援 -P、無 jq、或 JSON 格式非預期——都會讓整個 hook 靜默失效
[ -z "${FILE_PATH:-}" ] && warn "無法解析 file_path，本次未檢查（環境需有 jq 或支援 -P 的 grep）"

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

# 內容掃描（content / new_string，取前 200KB）：高信度 secret pattern
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
