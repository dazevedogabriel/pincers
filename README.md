# 🦀 Pincers

A segmented, multi-connection download manager written in Rust, rebuilt from the
reverse engineering of Internet Download Manager's engine
(see [docs/IDM_ANALYSIS.md](docs/IDM_ANALYSIS.md)).

## Usage

```bash
cargo build --release
```

### Graphical interface

Open `target/release/pincers.exe` with no arguments (double-click works), or
`pincers.exe gui <URL>...` to open the window and start downloading.
The look follows the Claude Design mock-up (dark theme, Geist fonts, custom title bar).

* **Top bar:** paste the URL and press Enter (or **Baixar**); **Partes 4 · 8 · 16 · 32**
  sets how many connections the next downloads use
* **Sidebar:** filters (Todos, Baixando, Pausados, Concluídos, Com falha) with counts,
  total speed with a chart, active connections, and **Opções**
* **List:** each download's real parts (Pincers splits dynamically, like IDM),
  speed, time remaining, status and a pause / resume / open folder button
* **Detail panel:** statistics, *Mapa do arquivo* (grid or bars, chosen in Opções),
  the speed chart, the parts table and Pausar/Retomar/Tentar novamente/Abrir arquivo,
  Abrir pasta and Remover; clicking the URL copies it
* below 1180 px wide the sidebar hides and the filters and Opções move to the
  list toolbar, as in the mock-up
* the list is saved to `%APPDATA%\Pincers\pincers.json`; downloads interrupted when
  the app closes come back as "Pausado" and resume where they stopped
* **Remover** takes the item off the list but doesn't delete files: the `.part`/`.pincers`
  of an unfinished download stay, and adding the same URL again resumes it

Fonts: Geist and Geist Mono (Vercel, SIL Open Font License, `assets/fonts/OFL-LICENSE.txt`)
are embedded in the executable. The logo comes from `assets/logo.svg`.

### Browser extension (Opera GX, Chrome, Firefox)

The extension in `extension/` works like IDM's "IDM Integration Module": when a
download of one of the configured types starts in the browser, it **cancels it in the browser
and copies the link** in a small window ("Download capturado"). Then just paste
(Ctrl+V) into Pincers's URL field. Pasting several links at once also works.

* hold **Alt** while clicking a link to let the browser download it
* right-click: **Copiar link para o Pincers** (links, images, videos) and
  **Copiar todos os links de arquivos para o Pincers** (whole page or selection)
* the toolbar icon opens the popup: on/off, the last captured links, the file
  types (IDM's default list) and sites to ignore

Build it with:

```bash
powershell -ExecutionPolicy Bypass -File extension/build.ps1
```

**Opera GX / Chrome:** open `opera://extensions` (or `chrome://extensions`), turn on
**Modo de desenvolvedor**, click **Carregar sem compactação** and pick `extension/dist/chrome`.

**Firefox:** regular Firefox only installs signed extensions permanently. To test,
open `about:debugging#/runtime/this-firefox` → **Carregar extensão temporária…** →
`extension/dist/firefox/manifest.json` (it goes away when Firefox closes). To install permanently,
sign `extension/dist/pincers-firefox.xpi` for free on addons.mozilla.org as
"distribuição própria" (unlisted) and install the signed `.xpi` you get back.

The extension's icons come from `assets/logo.svg`; after changing the logo, regenerate them with
`cargo run --release --example extension_icons` and run `build.ps1` again.

### Icon

The exe's icon (Explorer, desktop, shortcuts) and the window icon come from
`assets/logo.svg`. To change it, replace that file and run `cargo build --release`.
If Explorer keeps showing the old icon, that's its cache: run `ie4uinit.exe -show`
or restart Explorer.

### Command line

```bash
target/release/pincers.exe <URL> [-c 8] [-o folder] [-l 2M]
```

* `-c, --connections` maximum number of connections (default 8, like IDM)
* `-o, --output` destination folder
* `-l, --limit` speed limit (`500K`, `2M`…)
* `Ctrl+C` pauses. Run the same command again to resume where you left off.

## How it works (in short)

1. The first connection asks for `bytes=0-` and keeps downloading.
2. While there are free slots, Pincers picks the segment with the most
   *free space* (`remaining − speed × connect_time`) and splits it in half.
3. When a connection finishes, it immediately splits the largest remaining segment.
4. A connection that reaches its end absorbs the next segment if that one
   hasn't really started; otherwise it checks 64 bytes of overlap.
5. Everything is written straight into a preallocated `.part` file;
   progress is saved to `.pincers` every second.
6. If the server refuses extra connections (HTTP 429/503), Pincers settles on the
   number it accepts instead of insisting.
