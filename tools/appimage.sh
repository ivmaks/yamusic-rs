#!/bin/bash
# Собирает AppImage из уже собранного target/release/yamusic-rs: бинарник, ярлык и значки.
# Библиотеки не вшиваются, их даёт система: сборка на Ubuntu 22.04 работает с glibc 2.35
# и новее. appimagetool 1.9.1 скачивается сам и сверяется по sha256; его рантайм собран
# статически и не требует libfuse2.
# Результат: target/appimage/yamusic-rs-x86_64.AppImage.
set -euo pipefail
cd "$(dirname "$0")/.."

TOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
TOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
OUT=target/appimage
APPDIR="$OUT/yamusic-rs.AppDir"
TOOL="$OUT/appimagetool"

mkdir -p "$OUT"
if ! echo "$TOOL_SHA256  $TOOL" | sha256sum -c --status 2>/dev/null; then
  curl -fsSL -o "$TOOL" "$TOOL_URL"
  echo "$TOOL_SHA256  $TOOL" | sha256sum -c --quiet
  chmod +x "$TOOL"
fi

rm -rf "$APPDIR"
install -Dm755 target/release/yamusic-rs "$APPDIR/usr/bin/yamusic-rs"
install -Dm644 packaging/yamusic-rs.desktop "$APPDIR/usr/share/applications/yamusic-rs.desktop"
for size in 16 32 48 64 128 256; do
  install -Dm644 "packaging/icons/$size.png" "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps/yamusic-rs.png"
done
ln -s usr/bin/yamusic-rs "$APPDIR/AppRun"
ln -s usr/share/applications/yamusic-rs.desktop "$APPDIR/yamusic-rs.desktop"
ln -s usr/share/icons/hicolor/256x256/apps/yamusic-rs.png "$APPDIR/yamusic-rs.png"
ln -s yamusic-rs.png "$APPDIR/.DirIcon"

# Сам appimagetool — тоже AppImage: распаковка вместо FUSE работает и в контейнерах CI.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" "$OUT/yamusic-rs-x86_64.AppImage"
