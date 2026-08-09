// 兩階段解碼：登入前 big5，自動登入狀態機偵測到伺服器切換後改 utf-8
// （切換時機新舊 PTT 不同，見 docs/architecture.md「編碼」）。
// TextDecoder 的 stream 模式原生處理多位元組字跨 frame 切割；
// 非法位元組以 U+FFFD 替代顯示（non-fatal 預設行為），串流不中斷。
export class PttDecoder {
  private decoder = new TextDecoder("big5");

  decode(chunk: Uint8Array): string {
    return this.decoder.decode(chunk, { stream: true });
  }

  // 回傳值為 flush 舊 decoder 的殘留輸出（跨切換點的半個 big5 字以 U+FFFD 呈現），
  // 呼叫端應寫入畫面，避免位元組被靜默丟棄
  switchToUtf8(): string {
    const tail = this.decoder.decode();
    this.decoder = new TextDecoder("utf-8");
    return tail;
  }

  reset(): void {
    this.decoder = new TextDecoder("big5");
  }
}
