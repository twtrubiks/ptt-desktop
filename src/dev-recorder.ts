// dev 錄製（Plans.md P4 探勘）：把 telnet 濾層後的下行 bytes 以 JSONL 逐行
// POST 給 Vite dev server 的 /__ptt-rec middleware 落地（見 vite.config.ts），
// 保留 chunk 邊界供跨 frame／孤兒 byte 分析。
// 只在 import.meta.env.DEV 路徑被建構（main.ts）：release build 的 DEV 摺疊為
// false 後本模組整個被 tree-shake，dist 產物不含錄製路徑（驗證：grep dist 無
// __ptt-rec 字樣）。只錄下行——上行含帳密，絕不經過本模組。

const ENDPOINT = "/__ptt-rec";

// btoa 一次吃太長的字串會爆 call stack（fromCharCode 展開），分段組字串
function toBase64(bytes: Uint8Array): string {
  const STEP = 0x8000;
  let bin = "";
  for (let i = 0; i < bytes.length; i += STEP) {
    bin += String.fromCharCode(...bytes.subarray(i, i + STEP));
  }
  return btoa(bin);
}

export class DevRecorder {
  private seq = 0;
  private failed = false;
  private queue: Promise<void> = Promise.resolve();

  /** telnet 濾層後的下行 bytes，一個 chunk 一行（chunk 邊界即 WebSocket frame 邊界） */
  push(bytes: Uint8Array): void {
    this.send({ seq: this.seq++, t: Math.round(performance.now()), d: toBase64(bytes) });
  }

  /** 事件標記（connect / utf8-switch / close），供分析切段與判定 big5/utf-8 模式 */
  mark(event: string): void {
    this.send({ seq: this.seq++, t: Math.round(performance.now()), ev: event });
  }

  private send(line: Record<string, unknown>): void {
    if (this.failed) return;
    // promise 鏈序列化 POST，落地順序 = seq 順序；失敗即永久停錄——
    // 錄製是旁路，不重試不拋錯，任何失敗都不得干擾連線主流程
    this.queue = this.queue.then(() =>
      fetch(ENDPOINT, { method: "POST", body: JSON.stringify(line) }).then(
        (res) => {
          if (!res.ok) this.fail(`HTTP ${res.status}`);
        },
        (err) => this.fail(String(err)),
      ),
    );
  }

  private fail(reason: string): void {
    if (this.failed) return;
    this.failed = true;
    console.warn(`[dev-recorder] 錄製中止：${reason}（不影響連線）`);
  }
}
