// 滑鼠停在圖片連結上時浮動預覽：hover 延遲後掛一個 <img>，leave 即移除。
// 只處理直連圖檔與 imgur 單圖頁面；不自動掃描畫面（見設計討論：opt-in 才載圖）。

const SHOW_DELAY_MS = 250; // 滑鼠掃過連結不觸發，停住才載圖
const EDGE_MARGIN_PX = 12;

// 回傳可直接放進 <img src> 的網址；非圖片連結回傳 null。
export function resolveImageUrl(uri: string): string | null {
  let url: URL;
  try {
    url = new URL(uri);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (/\.(jpe?g|png|gif|webp)$/i.test(url.pathname)) return uri;
  // imgur 單圖頁面 → 直連圖檔；相簿 /a/、/gallery/ 無法改寫，交給點擊開瀏覽器
  if (url.hostname === "imgur.com" || url.hostname === "www.imgur.com") {
    const m = url.pathname.match(/^\/([A-Za-z0-9]{5,})$/);
    if (m) return `https://i.imgur.com/${m[1]}.jpg`;
  }
  return null;
}

export class ImagePreview {
  private timer: number | null = null;
  private img: HTMLImageElement | null = null;

  // 非圖片連結不動作；同一時間只顯示一張
  show(uri: string, event: MouseEvent): void {
    const src = resolveImageUrl(uri);
    if (!src) return;
    this.hide();
    this.timer = window.setTimeout(() => {
      this.timer = null;
      this.mount(src, event.clientX, event.clientY);
    }, SHOW_DELAY_MS);
  }

  hide(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.img?.remove();
    this.img = null;
  }

  private mount(src: string, mouseX: number, mouseY: number): void {
    const img = document.createElement("img");
    img.className = "image-preview";
    img.referrerPolicy = "no-referrer"; // imgur 等圖床對外站 Referer 可能回 403
    img.onerror = () => {
      if (this.img === img) this.hide();
    };
    // 依滑鼠所在象限往反方向錨定；搭配 CSS max 尺寸 45vw/45vh，必不超出視窗
    if (mouseX < window.innerWidth / 2) {
      img.style.left = `${mouseX + EDGE_MARGIN_PX}px`;
    } else {
      img.style.right = `${window.innerWidth - mouseX + EDGE_MARGIN_PX}px`;
    }
    if (mouseY < window.innerHeight / 2) {
      img.style.top = `${mouseY + EDGE_MARGIN_PX}px`;
    } else {
      img.style.bottom = `${window.innerHeight - mouseY + EDGE_MARGIN_PX}px`;
    }
    img.src = src;
    document.body.appendChild(img);
    this.img = img;
  }
}
