import { appendFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { defineConfig, type Plugin } from "vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// dev 錄製落地（Plans.md P4）：src/dev-recorder.ts 把 telnet 濾層後的下行 bytes
// 以 JSONL 逐行 POST 到這裡，append 進 recordings/（已 gitignore，原始錄檔含
// 個資、絕不進 repo）。apply: "serve" 限定只掛 dev server，與 build 產物無關。
function pttRecPlugin(): Plugin {
  let file = "";
  return {
    name: "ptt-rec",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__ptt-rec", (req, res) => {
        if (req.method !== "POST") {
          res.statusCode = 405;
          res.end();
          return;
        }
        let body = "";
        req.setEncoding("utf8");
        req.on("data", (chunk) => (body += chunk));
        req.on("end", () => {
          try {
            if (!file) {
              mkdirSync("recordings", { recursive: true });
              const stamp = new Date().toISOString().replace(/[:.]/g, "-");
              file = join("recordings", `rec-${stamp}.jsonl`);
            }
            appendFileSync(file, body + "\n");
            res.statusCode = 204;
          } catch {
            res.statusCode = 500;
          }
          res.end();
        });
      });
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  plugins: [pttRecPlugin()],
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
