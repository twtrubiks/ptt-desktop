#!/usr/bin/env bash
# Stop / PreCompact hook：提醒 cc:DOING 殘留。永遠 exit 0（純提醒，不阻擋）。
MODE="${1:-stop}"
PLANS="${CLAUDE_PROJECT_DIR:-.}/Plans.md"
[ -f "$PLANS" ] || exit 0

# 只比對任務標題行，避免誤報「狀態標記」說明表裡的 cc:DOING 字樣
LINES=$(grep -n '^###.*cc:DOING' "$PLANS" 2>/dev/null)
[ -z "$LINES" ] && exit 0

if [ "$MODE" = "compact" ]; then
  echo "⚠ Context 即將壓縮，Plans.md 仍有 cc:DOING 任務（壓縮後可能遺失工作脈絡）："
else
  echo "⚠ Plans.md 仍有 cc:DOING 任務未收尾（完成請改 cc:DONE，中斷請改回 cc:TODO 或 cc:BLOCKED）："
fi
echo "$LINES"
exit 0
