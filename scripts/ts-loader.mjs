// 讓 node 直接 import src/*.ts：Node 24 原生剝離型別，只差 Vite 風格的無副檔名
// 相對 import（./screen-grid → ./screen-grid.ts）。供 replay-check.mjs 使用。
import { registerHooks } from "node:module";

registerHooks({
  resolve(specifier, context, nextResolve) {
    try {
      return nextResolve(specifier, context);
    } catch (err) {
      if (specifier.startsWith(".")) return nextResolve(specifier + ".ts", context);
      throw err;
    }
  },
});
