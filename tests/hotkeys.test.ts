import { describe, expect, it } from "vitest";
import { HotkeyHandler, type HotkeyActions } from "../src/hotkeys";

function setup(hotkeys: Record<string, string> = {}) {
  const calls: string[] = [];
  const actions: HotkeyActions = {
    send: (sequence) => calls.push(`send:${sequence}`),
    reconnect: () => calls.push("reconnect"),
    toggleFullscreen: () => calls.push("toggleFullscreen"),
  };
  return { handler: new HotkeyHandler(hotkeys, actions), calls };
}

function keydown(key: string, modifiers: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return {
    type: "keydown",
    key,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    shiftKey: false,
    ...modifiers,
  } as KeyboardEvent;
}

describe("HotkeyHandler", () => {
  it("F11 切換全螢幕並攔截事件", () => {
    const { handler, calls } = setup();
    expect(handler.handleKeyEvent(keydown("F11"))).toBe(false);
    expect(calls).toEqual(["toggleFullscreen"]);
  });

  it("F11 優先於設定檔快捷鍵", () => {
    const { handler, calls } = setup({ F11: "sgossiping\r" });
    handler.handleKeyEvent(keydown("F11"));
    expect(calls).toEqual(["toggleFullscreen"]);
  });

  it("帶修飾鍵的 F11 不觸發全螢幕", () => {
    const { handler, calls } = setup();
    expect(handler.handleKeyEvent(keydown("F11", { ctrlKey: true }))).toBe(true);
    expect(calls).toEqual([]);
  });

  it("Ctrl+R 觸發重連", () => {
    const { handler, calls } = setup();
    expect(handler.handleKeyEvent(keydown("r", { ctrlKey: true }))).toBe(false);
    expect(calls).toEqual(["reconnect"]);
  });

  it("命中設定檔快捷鍵展開序列", () => {
    const { handler, calls } = setup({ F3: "sgossiping\r" });
    expect(handler.handleKeyEvent(keydown("F3"))).toBe(false);
    expect(calls).toEqual(["send:sgossiping\r"]);
  });

  it("未命中的按鍵放行給 xterm", () => {
    const { handler, calls } = setup();
    expect(handler.handleKeyEvent(keydown("a"))).toBe(true);
    expect(calls).toEqual([]);
  });

  it("Shift+特殊鍵不觸發該鍵的快捷鍵", () => {
    const { handler, calls } = setup({ F3: "sgossiping\r" });
    expect(handler.handleKeyEvent(keydown("F3", { shiftKey: true }))).toBe(true);
    expect(calls).toEqual([]);
  });

  it("需要 shift 的可列印字元快捷鍵照常觸發", () => {
    const { handler, calls } = setup({ "?": "h" });
    expect(handler.handleKeyEvent(keydown("?", { shiftKey: true }))).toBe(false);
    expect(calls).toEqual(["send:h"]);
  });
});
