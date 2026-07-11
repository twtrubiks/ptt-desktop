import { describe, expect, it } from "vitest";
import { resolveImageUrl } from "../src/image-preview";

describe("resolveImageUrl", () => {
  it("直連圖檔原樣回傳", () => {
    expect(resolveImageUrl("https://i.imgur.com/abc123.jpg")).toBe(
      "https://i.imgur.com/abc123.jpg",
    );
    expect(resolveImageUrl("https://example.com/photo.PNG")).toBe("https://example.com/photo.PNG");
    expect(resolveImageUrl("http://example.com/a.webp")).toBe("http://example.com/a.webp");
  });

  it("imgur 單圖頁面改寫為直連圖檔", () => {
    expect(resolveImageUrl("https://imgur.com/abc123")).toBe("https://i.imgur.com/abc123.jpg");
    expect(resolveImageUrl("https://www.imgur.com/abc123")).toBe("https://i.imgur.com/abc123.jpg");
  });

  it("imgur 相簿與 gallery 不改寫", () => {
    expect(resolveImageUrl("https://imgur.com/a/abc123")).toBeNull();
    expect(resolveImageUrl("https://imgur.com/gallery/abc123")).toBeNull();
  });

  it("非圖片連結回傳 null", () => {
    expect(resolveImageUrl("https://www.ptt.cc/bbs/Gossiping/index.html")).toBeNull();
    expect(resolveImageUrl("https://youtu.be/abc123XYZ_-")).toBeNull();
  });

  it("非 http(s) 或非法網址回傳 null", () => {
    expect(resolveImageUrl("file:///etc/passwd.png")).toBeNull();
    expect(resolveImageUrl("不是網址")).toBeNull();
  });
});
