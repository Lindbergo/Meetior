# Icons

Tauri expects the following files in this directory before bundling:

- `32x32.png`
- `128x128.png`
- `128x128@2x.png`
- `icon.icns` (macOS)
- `icon.ico` (Windows, unused for now)

Generate them from a single source PNG once you have a logo:

```sh
pnpm tauri icon path/to/source-1024.png
```

Until then, `pnpm tauri dev` works without bundling but `pnpm tauri build`
will fail without these files.
