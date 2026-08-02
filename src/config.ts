// 設定檔：~/.config/ptt-desktop/config.json，權限 600。
// 密碼明文為刻意的取捨（同 ~/.netrc 等級，見 docs/architecture.md）。
import { readTextFile, BaseDirectory } from "@tauri-apps/plugin-fs";

export interface AppConfig {
  username: string;
  password: string;
  kickOtherLogins: string;
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  hotkeys: Record<string, string>; // 按鍵 → 送出序列，例如 "F3": "sgossiping\r"
}

const DEFAULTS: AppConfig = {
  username: "",
  password: "",
  kickOtherLogins: "n",
  // 捆綁的衍生字型（見 src/styles.css 的 @font-face）：垂直度量已壓到 1em，
  // 彩繪比例正方。使用者自訂其他字型時，彩繪可能回到縱向拉長
  fontFamily: "PTT Square Mono",
  fontSize: 18,
  // 行高倍率。預設 1 = 字型自身的正方 cell：方塊字形（U+2588 等）字面恰為
  // 1em×1em，上下無縫，彩繪比例正確——彩繪保真優先，故預設不加行距。
  // 覺得長文偏擠者可自行調到 1.08 左右換行距，代價是 cell 一變高、方塊列間
  // 就露出等比例橫縫（Noto 原版 1.448 的 45% 黑縫即此成因，見 docs/architecture.md）
  lineHeight: 1,
  hotkeys: {},
};

const MAX_LINE_HEIGHT = 2;

// xterm 對 lineHeight < 1 直接拋例外（node_modules/@xterm/xterm/src/common/services/
// OptionsService.ts:177），手改設定檔填了非法值會讓終端整個建不起來，
// 故在此夾到合法範圍，不原樣往下傳
export function sanitizeLineHeight(value: unknown): number {
  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return DEFAULTS.lineHeight;
  return Math.min(MAX_LINE_HEIGHT, Math.max(1, n));
}

export interface LoadConfigResult {
  config: AppConfig;
  error: string | null; // 設定檔存在但解析失敗時的訊息；沒有設定檔為 null（正常情境）
}

export async function loadConfig(): Promise<LoadConfigResult> {
  let raw: string;
  try {
    raw = await readTextFile(".config/ptt-desktop/config.json", {
      baseDir: BaseDirectory.Home,
    });
  } catch {
    return { config: { ...DEFAULTS }, error: null }; // 沒有設定檔：跳過自動登入，全手動操作
  }
  try {
    const merged: AppConfig = { ...DEFAULTS, ...JSON.parse(raw) };
    return {
      config: { ...merged, lineHeight: sanitizeLineHeight(merged.lineHeight) },
      error: null,
    };
  } catch (error) {
    // 檔案存在但 JSON 壞掉：改用預設值，錯誤交由呼叫端顯示，不能靜默吞掉
    return { config: { ...DEFAULTS }, error: String(error) };
  }
}
