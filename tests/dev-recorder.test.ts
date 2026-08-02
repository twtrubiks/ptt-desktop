import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DevRecorder } from "../src/dev-recorder";

interface RecLine {
  seq: number;
  t: number;
  d?: string;
  ev?: string;
}

function bodies(mock: ReturnType<typeof vi.fn>): RecLine[] {
  return mock.mock.calls.map((call) => JSON.parse((call[1] as RequestInit).body as string));
}

function fromBase64(b64: string): number[] {
  return Array.from(atob(b64), (ch) => ch.charCodeAt(0));
}

// 讓 promise 鏈裡已 resolve 的 fetch 後續跑完
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchMock = vi.fn().mockResolvedValue({ ok: true });
  vi.stubGlobal("fetch", fetchMock);
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("DevRecorder：錄製行格式", () => {
  it("push 一個 chunk 一行，bytes 以 base64 可逆還原", async () => {
    const rec = new DevRecorder();
    const chunk = [0xa4, 0x40, 0x1b, 0x5b, 0x6d]; // Big5「一」+ ESC [ m
    rec.push(Uint8Array.from(chunk));
    await flush();

    const [line] = bodies(fetchMock);
    expect(fetchMock).toHaveBeenCalledWith(
      "/__ptt-rec",
      expect.objectContaining({ method: "POST" }),
    );
    expect(line.seq).toBe(0);
    expect(typeof line.t).toBe("number");
    expect(fromBase64(line.d!)).toEqual(chunk);
  });

  it("mark 寫入事件行，與 push 共用遞增 seq", async () => {
    const rec = new DevRecorder();
    rec.mark("connect");
    rec.push(Uint8Array.from([0x41]));
    rec.mark("utf8-switch");
    await flush();

    const lines = bodies(fetchMock);
    expect(lines.map((l) => l.seq)).toEqual([0, 1, 2]);
    expect(lines[0].ev).toBe("connect");
    expect(lines[1].d).toBeDefined();
    expect(lines[2].ev).toBe("utf8-switch");
  });

  it("超過 base64 分段邊界（0x8000）的大 chunk 仍可逆", async () => {
    const rec = new DevRecorder();
    const big = new Uint8Array(0x8000 + 3);
    for (let i = 0; i < big.length; i++) big[i] = i % 251;
    rec.push(big);
    await flush();

    expect(fromBase64(bodies(fetchMock)[0].d!)).toEqual(Array.from(big));
  });
});

describe("DevRecorder：序列化與失敗停錄", () => {
  it("前一個 POST 未完成時不發下一個（promise 鏈序列化）", async () => {
    let release!: (value: { ok: boolean }) => void;
    fetchMock.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));
    const rec = new DevRecorder();
    rec.push(Uint8Array.from([1]));
    rec.push(Uint8Array.from([2]));
    await flush();
    expect(fetchMock).toHaveBeenCalledTimes(1);

    release({ ok: true });
    await flush();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(bodies(fetchMock).map((l) => l.seq)).toEqual([0, 1]);
  });

  it("fetch reject 後永久停錄，不拋錯不重試", async () => {
    fetchMock.mockRejectedValueOnce(new Error("dev server 不在"));
    const rec = new DevRecorder();
    rec.push(Uint8Array.from([1]));
    await flush();
    rec.push(Uint8Array.from([2]));
    rec.mark("close");
    await flush();

    expect(fetchMock).toHaveBeenCalledTimes(1); // 後續 push/mark 不再打
    expect(console.warn).toHaveBeenCalledTimes(1);
  });

  it("HTTP 非 2xx 同樣停錄", async () => {
    fetchMock.mockResolvedValueOnce({ ok: false, status: 500 });
    const rec = new DevRecorder();
    rec.push(Uint8Array.from([1]));
    await flush();
    rec.push(Uint8Array.from([2]));
    await flush();

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
