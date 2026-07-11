// 設定檔：~/.config/ptt-desktop/config.json，權限 600。
// 密碼明文為刻意的取捨（同 ~/.netrc 等級，見 docs/architecture.md）。
import { readTextFile, BaseDirectory } from "@tauri-apps/plugin-fs";

export interface AppConfig {
  username: string;
  password: string;
  kickOtherLogins: string;
  fontFamily: string;
  fontSize: number;
  hotkeys: Record<string, string>; // 按鍵 → 送出序列，例如 "F3": "sgossiping\r"
}

const DEFAULTS: AppConfig = {
  username: "",
  password: "",
  kickOtherLogins: "n",
  fontFamily: "Noto Sans Mono CJK TC",
  fontSize: 18,
  hotkeys: {},
};

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
    return { config: { ...DEFAULTS, ...JSON.parse(raw) }, error: null };
  } catch (error) {
    // 檔案存在但 JSON 壞掉：改用預設值，錯誤交由呼叫端顯示，不能靜默吞掉
    return { config: { ...DEFAULTS }, error: String(error) };
  }
}
