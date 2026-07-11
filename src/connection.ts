// tauri-plugin-websocket 封裝：帶 Origin header 連 PTT 官方 WebSocket 端點，
// 下行 binary 交給呼叫端，並以 telnet NOP 做應用層 keepalive（防閒置踢出）。
// WebSocket 傳輸層的 ping/pong 由 plugin 底層處理，與 BBS 應用層閒置計時是兩回事。
import WebSocket from "@tauri-apps/plugin-websocket";

const PTT_WS_URL = "wss://ws.ptt.cc/bbs";
const PTT_ORIGIN = "https://term.ptt.cc";
const KEEPALIVE_INTERVAL_MS = 60_000;
// 首選 telnet NOP（FF F1），不進畫面；次選 NUL（0x00），E2E 實測後定案（見 docs/architecture.md）
const KEEPALIVE_BYTES = [0xff, 0xf1];

export interface ConnectionEvents {
  onBinary(data: Uint8Array): void;
  onClose(reason: string): void;
}

export class PttConnection {
  private ws: WebSocket | null = null;
  private keepalive: ReturnType<typeof setInterval> | null = null;
  private encoder = new TextEncoder();

  constructor(private events: ConnectionEvents) {}

  get connected(): boolean {
    return this.ws !== null;
  }

  async connect(): Promise<void> {
    if (this.ws) return;
    const ws = await WebSocket.connect(PTT_WS_URL, {
      headers: { Origin: PTT_ORIGIN },
    });
    this.ws = ws;
    ws.addListener((message) => {
      if (this.ws !== ws) return; // 舊連線的殘留訊息
      if (message.type === "Binary") {
        this.events.onBinary(Uint8Array.from(message.data));
      } else if (message.type === "Close") {
        this.handleClose(message.data?.reason || "連線已關閉");
      }
    });
    this.startKeepalive();
  }

  async disconnect(): Promise<void> {
    this.stopKeepalive();
    const ws = this.ws;
    this.ws = null;
    if (ws) await ws.disconnect().catch(() => {});
  }

  async send(bytes: Uint8Array | number[]): Promise<void> {
    if (!this.ws) throw new Error("尚未連線");
    try {
      await this.ws.send(Array.from(bytes));
    } catch (error) {
      this.handleClose(String(error));
      throw error;
    }
  }

  // 上行一律 TextEncoder 編成 UTF-8 後以 binary 送出（登入前只有 ASCII，不受編碼影響）
  sendText(text: string): void {
    void this.send(this.encoder.encode(text)).catch(() => {}); // 斷線已由 handleClose 通知
  }

  private handleClose(reason: string): void {
    if (!this.ws) return;
    this.stopKeepalive();
    const ws = this.ws;
    this.ws = null;
    void ws.disconnect().catch(() => {});
    this.events.onClose(reason);
  }

  private startKeepalive(): void {
    this.stopKeepalive();
    this.keepalive = setInterval(() => {
      void this.send(KEEPALIVE_BYTES).catch(() => {});
    }, KEEPALIVE_INTERVAL_MS);
  }

  private stopKeepalive(): void {
    if (this.keepalive !== null) {
      clearInterval(this.keepalive);
      this.keepalive = null;
    }
  }
}
