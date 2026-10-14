# Desktop icons

`source/icon.svg` is the editable source: the title-bar `BrandMark` cells and its three brand oranges,
centred with padding on a dark-grey tile (`#25252b`). The tile keeps the original 6.25% canvas inset
and 19.53125% outer corner radius. A subtle `#55555f` edge, 1 px wide at sizes >= 32 px, keeps the
shape visible on dark desktops; smaller frames use the lighter fill alone. Corners stay transparent.

From the repository root:

```sh
pnpm install --frozen-lockfile
pnpm icons:generate
```

The generator uses the pinned Tauri CLI (the same renderer as `pnpm tauri icon`). It checks the source
against `BrandMark.tsx`, `shell.css` and the colour tokens before replacing the desktop assets. Edit
the SVG when changing the brand, then regenerate; never hand-edit the PNG, ICO or ICNS outputs.

The master leaves 12.5% of the tile width on each side of the W. Each 14×10 grid unit is fitted to
whole pixels, rounding down where that padding and an integer scale cannot both fit. At 16/24 px,
scale 1 prioritizes legibility. Each coloured rectangle spans two grid units.

| Frame size (px) | Grid unit (px) | W width (px) | Horizontal padding inside tile, per side (px) |
| --------------- | -------------- | ------------ | --------------------------------------------- |
| 16              | 1              | 14           | 0                                             |
| 24              | 1              | 14           | 3.5                                           |
| 32              | 1              | 14           | 7                                             |
| 48              | 2              | 28           | 7                                             |
| 64              | 3              | 42           | 7                                             |
| 128             | 6              | 84           | 14                                            |
| 256             | 12             | 168          | 28                                            |
| 512             | 24             | 336          | 56                                            |
| 1024            | 48             | 672          | 112                                           |

The generator derives a temporary SVG viewport for every size, with integer mark offsets and a
constant-width tile edge. It assembles ICO and ICNS from the matching Tauri-rendered frames,
including both legacy ICNS RGB/mask pairs and Retina entries. All image payloads come from the CLI;
ICNS entries are sorted for reproducible output. Scratch files are removed.

Outputs: `icon.ico` (16, 24, 32, 48, 64, 256 px), `icon.icns`, the named PNGs from 16 to 256 px,
`icon.png` (512 px) and `app-icon.png` (1024 px). Only the desktop outputs are retained: this app
does not bundle Android, iOS or Appx packages.

`app/tauri.conf.json` supplies these assets to the native window and the macOS, Windows installer,
Linux desktop and AppImage bundles. No custom installer header/sidebar images or OS tray icon are
configured. Installed Windows/macOS notifications use the platform's application identity; Linux
requests the same `vibe-work` hicolor icon as the desktop entry. An uninstalled development run may
lack that icon theme entry. About, the title bar and the welcome screen use `BrandMark` directly.
