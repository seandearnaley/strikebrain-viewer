#!/usr/bin/env bash
# Builds brand/Strikebrain-Production-Pack from the mark's geometry and the
# palette below: every vector, PDF, PNG, platform icon, press image and the
# preview. `just brand` runs it; commit what it writes. The README in the
# pack is written by hand and left alone.
#
# Needs macOS 26 or later with Xcode 26 or later (actool compiles the Icon
# Composer icon, swift writes the PDFs), resvg and ImageMagick
# (`brew install resvg imagemagick`), and the Geist font, whose licence the
# pack carries.
set -euo pipefail
cd "$(dirname "$0")"
pack=Strikebrain-Production-Pack

# Brainstrike's mark: a true circle (centre 512,512, radius 434) and a
# six-point bolt cut out of it, one even-odd path on a 1024 grid.
mark="M512 78 A434 434 0 1 1 512 946 A434 434 0 1 1 512 78 Z M618 128 L251 558 L493 558 L358 892 L734 440 L524 440 Z"
ultramarine="#000CDE"
fuchsia="#FF2E93"
dark="#10131A"
light="#F7F8FA"
app_id=ai.brainstrike.viewer
font="${GEIST_FONT:-$HOME/Library/Fonts/Geist-Variable.ttf}"

for tool in resvg magick xcrun iconutil swift; do
    command -v "$tool" > /dev/null || { echo "build.sh needs $tool" >&2; exit 1; }
done
test -f "$font" || { echo "build.sh needs the Geist font at $font (set GEIST_FONT)" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for dir in vectors pdf png press previews platform; do rm -rf "${pack:?}/$dir"; done
mkdir -p "$pack"/{vectors,pdf,png,press,previews,licenses} \
    "$pack"/platform/{apple,windows,linux,web}

# An SVG on the 1024 grid: $1 title, $2 description, $3 body.
svg() {
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" role="img" aria-labelledby="title desc">\n'
    printf '  <title id="title">%s</title>\n  <desc id="desc">%s</desc>\n%s\n</svg>\n' "$1" "$2" "$3"
}
path() { printf '  <path fill="%s" fill-rule="evenodd" clip-rule="evenodd" d="%s"/>' "$1" "$mark"; }
cutout="A circle with a classic zigzag lightning bolt cut out of it."

v="$pack/vectors"
svg "Strikebrain color icon" "$cutout On ultramarine." \
    "  <rect width=\"1024\" height=\"1024\" fill=\"$ultramarine\"/>
$(path "$fuchsia")" > "$v/Strikebrain-Master.svg"
svg "Strikebrain fuchsia mark" "$cutout Transparent background." "$(path "$fuchsia")" > "$v/Strikebrain-Mark.svg"
svg "Strikebrain dark mark" "$cutout For light backgrounds." "$(path "$dark")" > "$v/Strikebrain-Monochrome-Dark.svg"
svg "Strikebrain white mark" "$cutout For dark backgrounds." "$(path "#FFFFFF")" > "$v/Strikebrain-Monochrome-Light.svg"
svg "Strikebrain on light" "$cutout On a light neutral." \
    "  <rect width=\"1024\" height=\"1024\" fill=\"$light\"/>
$(path "$fuchsia")" > "$v/Strikebrain-Light-Background.svg"
svg "Strikebrain on dark" "$cutout On a dark neutral." \
    "  <rect width=\"1024\" height=\"1024\" fill=\"$dark\"/>
$(path "$fuchsia")" > "$v/Strikebrain-Dark-Background.svg"
# The app icon for Windows and Linux: the master on a rounded tile, inset
# 64 units (6.25%), as their icon grids leave room around the shape.
svg "Strikebrain app icon" "$cutout On an ultramarine rounded tile." \
    "  <rect x=\"64\" y=\"64\" width=\"896\" height=\"896\" rx=\"200\" fill=\"$ultramarine\"/>
  <g transform=\"translate(64 64) scale(0.875)\">
$(path "$fuchsia")
  </g>" > "$v/Strikebrain-Tile.svg"

# PNG at $3 px from SVG $1 to $2.
png() { resvg --use-font-file "$font" -w "$3" -h "$3" "$1" "$2"; }

p="$pack/png"
for size in 16 24 32 48 64 128 256 512 1024 2048 4096; do
    png "$v/Strikebrain-Master.svg" "$p/Strikebrain-Master-$size.png" "$size"
done
for name in Mark Monochrome-Dark Monochrome-Light Light-Background Dark-Background Tile; do
    for size in 1024 2048; do png "$v/Strikebrain-$name.svg" "$p/Strikebrain-$name-$size.png" "$size"; done
done

# Vector PDFs, drawn with Core Graphics from the same geometry.
cat > "$work/pdf.swift" <<'SWIFT'
import CoreGraphics
import Foundation

let args = CommandLine.arguments
func color(_ hex: String) -> CGColor {
    let v = UInt32(hex.dropFirst(), radix: 16)!
    return CGColor(srgbRed: CGFloat(v >> 16 & 0xFF) / 255, green: CGFloat(v >> 8 & 0xFF) / 255,
                   blue: CGFloat(v & 0xFF) / 255, alpha: 1)
}
// Arguments: output, mark colour, background colour or "none".
var box = CGRect(x: 0, y: 0, width: 1024, height: 1024)
let context = CGContext(URL(fileURLWithPath: args[1]) as CFURL, mediaBox: &box, nil)!
context.beginPDFPage(nil)
// PDF's origin is bottom left; the mark's grid is top left.
context.translateBy(x: 0, y: 1024)
context.scaleBy(x: 1, y: -1)
if args[3] != "none" {
    context.setFillColor(color(args[3]))
    context.fill(box)
}
let mark = CGMutablePath()
mark.addEllipse(in: CGRect(x: 78, y: 78, width: 868, height: 868))
mark.addLines(between: [CGPoint(x: 618, y: 128), CGPoint(x: 251, y: 558), CGPoint(x: 493, y: 558),
                        CGPoint(x: 358, y: 892), CGPoint(x: 734, y: 440), CGPoint(x: 524, y: 440)])
mark.closeSubpath()
context.addPath(mark)
context.setFillColor(color(args[2]))
context.fillPath(using: .evenOdd)
context.endPDFPage()
context.closePDF()
SWIFT
swiftc -O -o "$work/pdf" "$work/pdf.swift" 2> /dev/null
d="$pack/pdf"
"$work/pdf" "$d/Strikebrain-Master.pdf" "$fuchsia" "$ultramarine"
"$work/pdf" "$d/Strikebrain-Mark.pdf" "$fuchsia" none
"$work/pdf" "$d/Strikebrain-Monochrome-Dark.pdf" "$dark" none
"$work/pdf" "$d/Strikebrain-Monochrome-Light.pdf" "#FFFFFF" none
"$work/pdf" "$d/Strikebrain-Light-Background.pdf" "$fuchsia" "$light"
"$work/pdf" "$d/Strikebrain-Dark-Background.pdf" "$fuchsia" "$dark"

# macOS: an Icon Composer icon, the mark over the ultramarine fill with
# Liquid Glass, as Brainstrike's desktop app has. actool compiles it into
# Assets.car, which macOS 26 and later show, and an .icns for older macOS;
# the bundle carries both.
a="$pack/platform/apple"
mkdir -p "$a/AppIcon.icon/Assets"
printf '<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">\n%s\n</svg>\n' \
    "$(path "$fuchsia")" > "$a/AppIcon.icon/Assets/01-foreground.svg"
cat > "$a/AppIcon.icon/icon.json" <<'JSON'
{
  "fill" : {
    "solid" : "srgb:0.00000,0.04706,0.87059,1.00000"
  },
  "groups" : [
    {
      "layers" : [
        {
          "image-name" : "01-foreground.svg",
          "name" : "01-foreground"
        }
      ],
      "shadow" : {
        "kind" : "neutral",
        "opacity" : 0.5
      },
      "translucency" : {
        "enabled" : true,
        "value" : 0.2
      }
    }
  ],
  "supported-platforms" : {
    "squares" : [
      "macOS"
    ]
  }
}
JSON
mkdir "$work/actool"
# --app-icon names the icon by its file name.
DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}" xcrun actool "$a/AppIcon.icon" \
    --compile "$work/actool" --output-partial-info-plist "$work/actool/info.plist" --app-icon AppIcon \
    --include-all-app-icons --enable-on-demand-resources NO --development-region en \
    --target-device mac --minimum-deployment-target 11.0 --platform macosx > /dev/null
cp "$work/actool/Assets.car" "$work/actool/AppIcon.icns" "$a/"
# The icon as Finder shows it on this Mac, from a scratch app that carries
# both, for previews and listings.
probe="$work/Probe.app/Contents"
mkdir -p "$probe/Resources" "$probe/MacOS"
cp "$a/Assets.car" "$a/AppIcon.icns" "$probe/Resources/"
# Finder badges an app it cannot run.
cp /usr/bin/true "$probe/MacOS/Probe"
cat > "$probe/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$app_id.brand-probe.$$</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIconName</key><string>AppIcon</string>
  <key>CFBundleExecutable</key><string>Probe</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
cat > "$work/render.swift" <<'SWIFT'
import AppKit

let args = CommandLine.arguments
let icon = NSWorkspace.shared.icon(forFile: args[1])
let side = 1024
let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side,
                              bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                              colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
icon.draw(in: NSRect(x: 0, y: 0, width: side, height: side))
NSGraphicsContext.restoreGraphicsState()
try! bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
SWIFT
swiftc -O -o "$work/render" "$work/render.swift" 2> /dev/null
"$work/render" "$work/Probe.app" "$a/Strikebrain-macOS-1024.png"
# A flat, opaque square: the source Xcode's other icon workflows take.
magick "$p/Strikebrain-Master-1024.png" -background "$ultramarine" -alpha remove -alpha off \
    "$a/Strikebrain-Flat-1024-RGB.png"

# Windows: the tile, drawn at each size Explorer, the taskbar and the title
# bar pick from, in one .ico.
w="$pack/platform/windows"
mkdir -p "$w/png"
ico=()
for size in 16 20 24 32 40 48 64 96 128 256; do
    png "$v/Strikebrain-Tile.svg" "$w/png/Strikebrain-$size.png" "$size"
    ico+=("$w/png/Strikebrain-$size.png")
done
magick "${ico[@]}" "$w/Strikebrain.ico"

# Linux: the tile in the hicolor theme at each size, scalable, and a
# single-colour symbolic icon, named by the app's id.
l="$pack/platform/linux/hicolor"
for size in 16 22 24 32 48 64 128 256 512; do
    mkdir -p "$l/${size}x${size}/apps"
    png "$v/Strikebrain-Tile.svg" "$l/${size}x${size}/apps/$app_id.png" "$size"
done
mkdir -p "$l/scalable/apps" "$l/symbolic/apps"
cp "$v/Strikebrain-Tile.svg" "$l/scalable/apps/$app_id.svg"
printf '<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 1024 1024">\n%s\n</svg>\n' \
    "$(path "#2E3436")" > "$l/symbolic/apps/$app_id-symbolic.svg"

# Web: favicons, the Apple touch icon and web-app icons. The maskable one
# keeps the circle inside the central 80% a mask may leave.
web="$pack/platform/web"
cp "$v/Strikebrain-Master.svg" "$web/favicon.svg"
for size in 16 32 48; do png "$v/Strikebrain-Master.svg" "$web/favicon-${size}x${size}.png" "$size"; done
magick "$web/favicon-16x16.png" "$web/favicon-32x32.png" "$web/favicon-48x48.png" "$web/favicon.ico"
png "$v/Strikebrain-Master.svg" "$web/apple-touch-icon.png" 180
png "$v/Strikebrain-Master.svg" "$web/pwa-192.png" 192
png "$v/Strikebrain-Master.svg" "$web/pwa-512.png" 512
svg "Strikebrain maskable icon" "$cutout Padded for masks." \
    "  <rect width=\"1024\" height=\"1024\" fill=\"$ultramarine\"/>
  <g transform=\"translate(128 128) scale(0.75)\">
$(path "$fuchsia")
  </g>" > "$web/pwa-maskable.svg"
png "$web/pwa-maskable.svg" "$web/pwa-maskable-512.png" 512

# Press: artwork alone at 2048 px, and social images with the name in Geist.
pr="$pack/press"
cp "$p/Strikebrain-Mark-2048.png" "$pr/Strikebrain-Mark-Transparent-2048.png"
cp "$p/Strikebrain-Monochrome-Dark-2048.png" "$pr/Strikebrain-Monochrome-Dark-Transparent-2048.png"
cp "$p/Strikebrain-Monochrome-Light-2048.png" "$pr/Strikebrain-Monochrome-Light-Transparent-2048.png"
cp "$p/Strikebrain-Master-2048.png" "$pr/Strikebrain-Master-Press-2048.png"
cp "$p/Strikebrain-Light-Background-2048.png" "$pr/Strikebrain-Light-Background-Press-2048.png"
cp "$p/Strikebrain-Dark-Background-2048.png" "$pr/Strikebrain-Dark-Background-Press-2048.png"
# $1 file, $2 width, $3 height, $4 background, $5 mark colour, $6 text colour.
social() {
    local h=$3 side=$(( $3 * 46 / 100 ))
    local x=$(( $2 * 9 / 100 )) y=$(( ($3 - side) / 2 ))
    local scale; scale=$(echo "scale=6; $side / 1024" | bc)
    local tx=$(( x + side + $2 * 5 / 100 )) size=$(( h * 13 / 100 ))
    cat > "$work/social.svg" <<SOCIAL
<svg xmlns="http://www.w3.org/2000/svg" width="$2" height="$3" viewBox="0 0 $2 $3">
  <rect width="$2" height="$3" fill="$4"/>
  <g transform="translate($x $y) scale($scale)">$(path "$5")</g>
  <text x="$tx" y="$(( h / 2 - size / 10 ))" font-family="Geist" font-weight="700" font-size="$size" fill="$6" letter-spacing="-2">Strikebrain</text>
  <text x="$tx" y="$(( h / 2 + size ))" font-family="Geist" font-weight="400" font-size="$size" fill="$6" opacity="0.72" letter-spacing="-2">Viewer</text>
</svg>
SOCIAL
    resvg --use-font-file "$font" --skip-system-fonts "$work/social.svg" "$1"
}
social "$pr/Strikebrain-Social-Color-1200x630.png" 1200 630 "$ultramarine" "$fuchsia" "#FFFFFF"
social "$pr/Strikebrain-Social-Dark-1200x630.png" 1200 630 "$dark" "$fuchsia" "#FFFFFF"
social "$pr/Strikebrain-Social-Light-1200x630.png" 1200 630 "$light" "$fuchsia" "$dark"
social "$pr/Strikebrain-GitHub-Social-1280x640.png" 1280 640 "$ultramarine" "$fuchsia" "#FFFFFF"

# The preview: the master, the small sizes enlarged pixel for pixel, the
# rendered macOS, Windows and Linux icons, and the marks on their grounds.
pv="$work/preview"
mkdir "$pv"
label() { magick -size "$2"x40 xc:none -font "$font" -pointsize 20 -fill "#4B5563" -gravity center -annotate +0+0 "$3" "$1"; }
for size in 16 24 32 64; do
    png "$v/Strikebrain-Master.svg" "$pv/$size.png" "$size"
    magick "$pv/$size.png" -filter point -resize 144x144 "$pv/$size-big.png"
    label "$pv/$size-label.png" 144 "$size px"
    magick "$pv/$size-big.png" "$pv/$size-label.png" -background none -append "$pv/$size-cell.png"
done
magick "$pv"/16-cell.png "$pv"/24-cell.png "$pv"/32-cell.png "$pv"/64-cell.png -background none -splice 32x0 +append "$pv/small.png"
for pair in "macOS:$a/Strikebrain-macOS-1024.png" "Windows:$w/png/Strikebrain-256.png" "Linux:$l/256x256/apps/$app_id.png"; do
    name=${pair%%:*}; file=${pair#*:}
    magick "$file" -resize 176x176 "$pv/$name-icon.png"
    label "$pv/$name-label.png" 176 "$name"
    magick "$pv/$name-icon.png" "$pv/$name-label.png" -background none -append "$pv/$name-cell.png"
done
magick "$pv/macOS-cell.png" "$pv/Windows-cell.png" "$pv/Linux-cell.png" -background none -splice 32x0 +append "$pv/platforms.png"
magick "$p/Strikebrain-Mark-1024.png" -resize 116x116 -background white -gravity center -extent 140x140 "$pv/on-white.png"
magick "$p/Strikebrain-Monochrome-Dark-1024.png" -resize 116x116 -background "$light" -gravity center -extent 140x140 "$pv/mono-dark.png"
magick "$p/Strikebrain-Monochrome-Light-1024.png" -resize 116x116 -background "$dark" -gravity center -extent 140x140 "$pv/mono-light.png"
magick "$p/Strikebrain-Mark-1024.png" -resize 116x116 -background "$dark" -gravity center -extent 140x140 "$pv/on-dark.png"
magick "$pv/on-white.png" "$pv/mono-dark.png" "$pv/mono-light.png" "$pv/on-dark.png" -background none -splice 32x0 +append "$pv/variants.png"
magick "$pv/small.png" \( -size 1x28 xc:none \) "$pv/platforms.png" \( -size 1x28 xc:none \) "$pv/variants.png" \
    -background none -gravity west -append "$pv/right.png"
magick -size 1400x760 xc:"#F2F3F5" \
    \( "$p/Strikebrain-Master-512.png" \) -geometry +40+128 -composite \
    \( "$pv/right.png" \) -geometry +580+100 -composite \
    -font "$font" -fill "#111111" -pointsize 30 -annotate +40+58 "Strikebrain Viewer - production icon" \
    -fill "#4B5563" -pointsize 18 -annotate +40+92 "Brainstrike's mark in fuchsia $fuchsia on ultramarine $ultramarine." \
    "$pack/previews/Strikebrain-Production-Preview.png"

# The licence of the font in the press images and the preview.
[ -f "$pack/licenses/Geist-OFL.txt" ] || curl -fsSL https://raw.githubusercontent.com/vercel/geist-font/main/OFL.txt -o "$pack/licenses/Geist-OFL.txt"

# Inventory and checksums.
(
    cd "$pack"
    find . -type f ! -name SHA256SUMS.txt ! -name manifest.json ! -name .DS_Store | sed 's#^\./##' | LC_ALL=C sort > "$work/files"
    {
        printf '{\n  "identity": "Strikebrain Viewer",\n  "edition": "Production identity pack",\n'
        printf '  "derived_from": "Brainstrike production identity pack, mark geometry unchanged",\n'
        printf '  "date": "%s",\n' "$(date +%F)"
        printf '  "palette": {\n    "ultramarine": "%s",\n    "fuchsia": "%s",\n    "dark_neutral": "%s",\n    "light_neutral": "%s"\n  },\n' \
            "$ultramarine" "$fuchsia" "$dark" "$light"
        printf '  "master_geometry": {\n    "viewBox": [0, 0, 1024, 1024],\n    "circle_center": [512, 512],\n    "circle_radius": 434,\n'
        printf '    "bolt_vertices": [[618, 128], [251, 558], [493, 558], [358, 892], [734, 440], [524, 440]],\n'
        printf '    "app_tile": {"inset": 64, "size": 896, "corner_radius": 200}\n  },\n'
        printf '  "files": [\n'
        first=1
        while IFS= read -r f; do
            dims=""
            case "$f" in *.png) dims=$(magick identify -format '%wx%h' "$f[0]");; esac
            [ $first = 1 ] || printf ',\n'
            first=0
            printf '    {"path": "%s", "bytes": %s%s}' "$f" "$(stat -f %z "$f")" "${dims:+, \"pixels\": \"$dims\"}"
        done < "$work/files"
        printf '\n  ]\n}\n'
    } > manifest.json
    tr "\n" "\0" < "$work/files" | xargs -0 shasum -a 256 > SHA256SUMS.txt
)
echo "wrote $pack ($(wc -l < "$pack/SHA256SUMS.txt" | tr -d ' ') files)"
