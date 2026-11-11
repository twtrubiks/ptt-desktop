#!/bin/bash
# 把 release 版 ptt-gpui 打成 AppImage。
#
# 用法：
#   APPIMAGETOOL=/path/to/appimagetool RUNTIME=/path/to/runtime-x86_64 \
#     scripts/build-appimage.sh
#
# 兩個工具都從 https://github.com/AppImage 下載（appimagetool continuous、
# type2-runtime continuous）。APPIMAGETOOL 可以是 .AppImage 本體（需要 FUSE）
# 或 `--appimage-extract` 解開後的 squashfs-root/AppRun（不需要 FUSE）。
#
# 只包執行檔本身，不捆綁系統函式庫：ldd 顯示的依賴（libssl3、libxkbcommon、xcb、
# zlib、zstd）任何桌面發行版都有。字型與 Vulkan 驅動一律用目標機器的；字型需要
# Noto Sans Mono CJK TC（或在設定檔 fontFamily 指定別的等寬 CJK 字型）。
set -euo pipefail

HERE=$(cd "$(dirname "$0")/.." && pwd)
APPIMAGETOOL=${APPIMAGETOOL:-appimagetool}
ARCH=${ARCH:-x86_64}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$HERE/Cargo.toml" | head -1)
OUT_DIR="$HERE/target/appimage"
APPDIR="$OUT_DIR/ptt-gpui.AppDir"
OUT="$OUT_DIR/ptt-gpui-$VERSION-$ARCH.AppImage"

( cd "$HERE" && cargo build --release )

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
  "$APPDIR/usr/share/icons/hicolor/128x128/apps"
cp "$HERE/target/release/ptt-gpui" "$APPDIR/usr/bin/"
cp "$HERE/packaging/ptt-gpui.desktop" "$APPDIR/"
cp "$HERE/packaging/ptt-gpui.desktop" "$APPDIR/usr/share/applications/"
cp "$HERE/packaging/ptt-gpui.png" "$APPDIR/"
cp "$HERE/packaging/ptt-gpui.png" "$APPDIR/usr/share/icons/hicolor/128x128/apps/"
cp "$HERE/packaging/ptt-gpui.png" "$APPDIR/.DirIcon"
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
exec "$HERE/usr/bin/ptt-gpui" "$@"
RUN
chmod +x "$APPDIR/AppRun"

RUNTIME_ARGS=()
if [ -n "${RUNTIME:-}" ]; then
  RUNTIME_ARGS=(--runtime-file "$RUNTIME")
fi
rm -f "$OUT"
ARCH=$ARCH "$APPIMAGETOOL" "${RUNTIME_ARGS[@]}" "$APPDIR" "$OUT"
ls -l "$OUT"
