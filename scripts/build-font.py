#!/usr/bin/env python3
"""產製 PTT Square Mono：修正 Noto Sans Mono CJK TC 垂直度量的衍生字型。

為什麼需要：Noto Sans Mono CJK TC 的 hhea ascent/descent 為 1160/-288（行高
1.448em），xterm 以 fontBoundingBox 當每列高度，80×24 畫面被縱向拉長 45%、
彩繪比例失真。xterm 6 禁止 lineHeight < 1、WebKitGTK 不支援 ascent-override，
唯一不動文字間距的解法是把字型本身的垂直度量壓到 1em（= 全形字寬）再捆綁。

做法：hhea 與 OS/2 usWin 度量改用該字型自己的 sTypo 度量（880/-120，加總
恰為 1000 = 1em）；字面設計未動，只是行高不再帶 CJK 上下伸部的保留空間。

另砍掉韓文諺文字集（11,675 個 codepoint，佔 10.89 MB 中的 1.54 MB）：PTT 是
Big5 站台，畫面內容源自 Big5 編碼（UTF-8 模式也只是把同一份 Big5 內容轉碼送出），
諺文不在 Big5／UAO 的映射域，送不出來。砍字集的代價是缺字時 WebView 會 fallback
到系統字型，而系統字型的垂直度量不是 1em——該列會跑版（等於局部重現本檔要修的
問題），所以只砍「協定上不可能出現」的字集，不砍任何 Big5 可解出的字。

授權：Noto 為 SIL OFL 1.1，"Noto" 是 Reserved Font Name，衍生版必須改名
（PTT Square Mono）；OFL 全文與衍生說明見 src/assets/OFL.txt。

用法（需 fonttools + brotli，pyenv grpc 環境已有）：
    pyenv exec python scripts/build-font.py
輸出：src/assets/ptt-square-mono.woff2

產製紀錄（2026-08-01 初版，2026-08-02 加子集化）：來源 Debian fonts-noto-cjk
1:20240730+repack1-1，字型版本 2.004（name ID 5），fonttools 4.63.0。
換來源版本重跑時，輸出會隨上游字型內容變動，須重驗度量與名稱後更新本紀錄。
"""

from pathlib import Path

from fontTools import subset, ttLib

SRC = "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"
OUT = Path(__file__).resolve().parent.parent / "src" / "assets" / "ptt-square-mono.woff2"
OLD_FAMILY = "Noto Sans Mono CJK TC"
NEW_FAMILY = "PTT Square Mono"
OLD_PS = "NotoSansMonoCJKtc"
NEW_PS = "PTTSquareMono"

# 韓文諺文的全部 Unicode 區段（含相容字母與半形）。這是唯一被砍的字集：
# Big5／UAO 無任何 codepoint 映射到此範圍，PTT 協定上送不出來
HANGUL_RANGES = [
    (0x1100, 0x11FF),  # Hangul Jamo
    (0x3130, 0x318F),  # Hangul Compatibility Jamo
    (0xA960, 0xA97F),  # Hangul Jamo Extended-A
    (0xAC00, 0xD7AF),  # Hangul Syllables（大宗，11,172 字）
    (0xD7B0, 0xD7FF),  # Hangul Jamo Extended-B
    (0xFFA0, 0xFFDF),  # Halfwidth Hangul
]


def pick_font(collection: ttLib.TTCollection) -> ttLib.TTFont:
    for font in collection.fonts:
        family = font["name"].getDebugName(16) or font["name"].getDebugName(1)
        if family == OLD_FAMILY:
            return font
    raise SystemExit(f"{SRC} 內找不到 {OLD_FAMILY}")


def fix_vertical_metrics(font: ttLib.TTFont) -> None:
    upm = font["head"].unitsPerEm
    os2 = font["OS/2"]
    ascent, descent = os2.sTypoAscender, os2.sTypoDescender
    # 分配比例（880/-120）是此字型的設計值；若日後換字型來源，加總不為 1em 時
    # 必須人工重新確認分配，不能矇著調
    if ascent - descent != upm:
        raise SystemExit(f"sTypo 度量 {ascent}/{descent} 加總 ≠ {upm}，分配需人工確認")
    hhea = font["hhea"]
    hhea.ascent = ascent
    hhea.descent = descent
    hhea.lineGap = 0
    os2.usWinAscent = ascent
    os2.usWinDescent = -descent
    os2.sTypoLineGap = 0


def drop_hangul(font: ttLib.TTFont) -> None:
    """子集化：保留 cmap 中除諺文外的全部 codepoint。

    保留設定的理由（都不是預設值，改動前先讀）：
    - name_IDs / name_legacy：rename() 依賴 nameID 16/17，subset 預設只留 0-6 會砍掉
    - notdef_outline：PUA 半字的 .notdef 行為是 duotone.ts 的既有前提（用 conceal 隱藏），
      不能讓 .notdef 從有輪廓變空白
    - layout_features：等寬字型的 feature 影響字形選擇，保守全留
    - desubroutinize：子集後 CFF subroutine 大多失去共用價值，展開反而讓 brotli 壓得更小
      （實測 9.35 MB vs 保留 subroutine 的 9.64 MB）
    """
    cmap = set(font.getBestCmap())
    keep = {cp for cp in cmap if not any(lo <= cp <= hi for lo, hi in HANGUL_RANGES)}
    dropped = len(cmap) - len(keep)
    if dropped == 0:
        raise SystemExit("cmap 內找不到諺文字集，來源字型可能已變動，請重新確認")
    options = subset.Options()
    options.name_IDs = ["*"]
    options.name_legacy = True
    options.notdef_outline = True
    options.layout_features = ["*"]
    options.desubroutinize = True
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(unicodes=keep)
    subsetter.subset(font)
    print(f"子集化：cmap {len(cmap)} → {len(keep)}（砍 {dropped} 個諺文 codepoint）")


def rename(font: ttLib.TTFont) -> None:
    # OFL Reserved Font Name：名稱類 record（1/3/4/6/16/17）不得含 "Noto"；
    # copyright 與授權敘述（0/13/14）保留原文
    for record in font["name"].names:
        if record.nameID in (1, 3, 4, 6, 16, 17):
            value = record.toUnicode().replace(OLD_FAMILY, NEW_FAMILY).replace(OLD_PS, NEW_PS)
            record.string = value.encode(record.getEncoding())
    cff = font["CFF "].cff
    cff.fontNames[0] = f"{NEW_PS}-Regular"
    top = cff.topDictIndex[0]
    top.FamilyName = NEW_FAMILY
    top.FullName = f"{NEW_FAMILY} Regular"


def main() -> None:
    collection = ttLib.TTCollection(SRC)
    font = pick_font(collection)
    # 順序固定：subset 會重算 OS/2 的 ulUnicodeRange 等欄位，度量修正與改名
    # 都放在它之後，確保最終寫出的是我們指定的值
    drop_hangul(font)
    fix_vertical_metrics(font)
    rename(font)
    font.flavor = "woff2"
    OUT.parent.mkdir(parents=True, exist_ok=True)
    font.save(str(OUT))
    size_mb = OUT.stat().st_size / 1024 / 1024
    print(f"已輸出 {OUT}（{size_mb:.1f} MB）")


if __name__ == "__main__":
    main()
