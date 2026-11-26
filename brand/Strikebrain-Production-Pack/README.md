# Strikebrain Viewer production identity pack

Brainstrike's mark, unchanged, in Strikebrain Viewer's colours: the circle
is fuchsia instead of chartreuse, on the same ultramarine. `../build.sh`
writes every file here from the geometry and palette below (`just brand`);
this README is the only file written by hand.

## Start here

- `vectors/Strikebrain-Master.svg`: the full-colour square icon.
- `vectors/Strikebrain-Tile.svg`: the app icon for Windows and Linux, the
  master on a rounded ultramarine tile.
- `vectors/Strikebrain-Mark.svg`: the fuchsia mark on a transparent background.
- `vectors/Strikebrain-Monochrome-Dark.svg` and `-Monochrome-Light.svg`: one-colour
  marks for light and dark backgrounds.
- `vectors/Strikebrain-Light-Background.svg` and `-Dark-Background.svg`: the mark
  on neutral grounds.
- `pdf/`: the same artwork as vector PDFs, drawn with Core Graphics.
- `png/`: the master at 16, 24, 32, 48, 64, 128, 256, 512, 1024, 2048 and 4096
  pixels; the other artwork at 1024 and 2048.
- `press/`: artwork alone at 2048 pixels, and social images with the name set in
  Geist: 1200 x 630 in colour, dark and light, and 1280 x 640 for a GitHub
  repository's social preview.
- `platform/`: the icons each platform installs (below).
- `previews/Strikebrain-Production-Preview.png`: the artwork, the small sizes
  pixel for pixel, and each platform's icon as rendered.
- `manifest.json` and `SHA256SUMS.txt`: every file, its size and pixels, and checksums.

## Colour

Digital sRGB is the source of truth.

| Colour | HEX | RGB |
| --- | --- | --- |
| Ultramarine | #000CDE | 0, 12, 222 |
| Fuchsia | #FF2E93 | 255, 46, 147 |
| Dark neutral | #10131A | 16, 19, 26 |
| Light neutral | #F7F8FA | 247, 248, 250 |
| White | #FFFFFF | 255, 255, 255 |

Fuchsia against ultramarine is a strong hue contrast but a modest lightness
contrast, about 2.9:1. Brainstrike's chartreuse is far lighter. The mark
still reads at 16 pixels (see the preview), and the monochrome marks are
there for places that need more contrast.

As with Brainstrike's pack, the PDFs and SVGs carry the RGB palette, not a
CMYK or spot-ink specification: a printer should convert them with the
right profile and proof the result.

## Geometry

- Every standard SVG uses `viewBox="0 0 1024 1024"`.
- The circle is centred at 512,512 with radius 434.
- The bolt's six points: 618,128; 251,558; 493,558; 358,892; 734,440; 524,440.
- Circle and bolt are one even-odd path, so the bolt is a cutout, not a painted shape.
- The app tile is inset 64 units (6.25%) on every side, 896 square, with
  200-unit corners; the master is scaled by 0.875 into it.

Keep the mark square and the bolt as it is; no gradients, outlines or
stretching. Leave clear space at least equal to the master's 78-unit margin.

## Platforms

### macOS

`platform/apple/AppIcon.icon` is an Icon Composer icon: the fuchsia mark as
its one layer, over the ultramarine fill, with Liquid Glass translucency,
as Brainstrike's desktop app has. `actool` compiles it into:

- `Assets.car`: macOS 26 and later show it with Liquid Glass, in light, dark
  and tinted appearances; macOS 11 to 15 read its flattened 16 to 1024 pixel
  images. The app's Info.plist names it with `CFBundleIconName`.
- `AppIcon.icns`: the `CFBundleIconFile` fallback, 16 to 256 pixels, as actool writes it.

`Strikebrain-macOS-1024.png` is the icon as Finder rendered it when the pack
was built; `Strikebrain-Flat-1024-RGB.png` is an opaque flat square for other
icon workflows. `just bundle` copies `Assets.car` and `AppIcon.icns` into the
app. To change the icon, open `AppIcon.icon` in Icon Composer
(Xcode > Open Developer Tool), or edit the palette in `../build.sh`, then run
`just brand`.

### Windows

`platform/windows/Strikebrain.ico` holds the tile drawn at 16, 20, 24, 32,
40, 48, 64, 96, 128 and 256 pixels, each from the vector, not scaled from a
larger one; `png/` has the same images. The build script compiles it into
`strikebrain-viewer.exe` as icon 1, which Explorer, the taskbar and the window
show, and Inno Setup uses it for the installer.

### Linux

`platform/linux/hicolor` is laid out as the icon theme expects and named by
the app's id, `ai.brainstrike.viewer`, as its desktop entry's `Icon=` line
asks: PNGs at 16, 22, 24, 32, 48, 64, 128, 256 and 512 pixels, the scalable
SVG, and a one-colour symbolic icon. The `.deb` and the `.tar.gz` install the
whole tree under `share/icons`.

### Web

`platform/web` holds `favicon.svg`, `favicon.ico` (16, 32 and 48 pixels) and
PNGs, a 180 pixel Apple touch icon, and 192 and 512 pixel web-app icons,
with a maskable 512 whose circle stays inside the central 80%.

## What was checked

- Each PNG was rendered from its SVG at its own size with resvg; the preview
  shows 16, 24, 32 and 64 pixels enlarged.
- `actool` compiled the Icon Composer icon on macOS 27 with Xcode 27; the
  app bundle built from it showed the icon in Finder with Liquid Glass.
- The PDFs were rasterised with macOS to check orientation and the cutout.
- The `.ico` was read back: ten 32-bit images, 16 to 256 pixels.

Not checked: the icon on macOS 11 to 15, on a Windows desktop, or in a Linux
desktop's launcher. The press images were not checked in a social network's
card preview.

## Licences

The press images and the preview set the name in Geist, under the SIL Open
Font License 1.1 (`licenses/Geist-OFL.txt`).
