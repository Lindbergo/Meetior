# Icons

Placeholder solid-blue PNGs are committed so `cargo check` / `pnpm tauri dev`
work out of the box. They are intentionally ugly. Replace before shipping.

Tauri expects:

- `32x32.png`
- `128x128.png`
- `128x128@2x.png`
- `icon.icns` (macOS bundling, not committed — required for `pnpm tauri build`)
- `icon.ico` (Windows, unused)

Generate them all from one source PNG with:

```sh
pnpm tauri icon path/to/source-1024.png
```
