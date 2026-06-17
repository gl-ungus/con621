# con621

A fast, lightweight console (TUI) client for [e621](https://e621.net) built in Rust.

> **Note:** e621 hosts adult/NSFW content. con621 defaults to the `safe` rating filter, but the full catalog is accessible. Use responsibly and in line with e621's terms.

## Features

- Search posts with full e621 tag syntax (`tag1 tag2`, `-tag`, `~tag`)
- Sort by score, favorites, newest, or oldest
- Filter by rating (safe / questionable / explicit / all)
- **Inline image preview** in graphics-capable terminals, with a half-block text fallback elsewhere (toggle with `i`)
- **Animated GIF and video preview** played as frames in the terminal, **with sound** for clips that have an audio track
- Vim-style navigation (`j`/`k`/`h`/`l`)
- View detailed post info (tags, score, artists, sources, description)
- Open posts in your browser
- Download files to your Downloads folder
- Pagination support

## Installation

### From crates.io (when i fix it)

```
cargo install con621
```

### From source

```
git clone https://github.com/gitlab-stack/con621
cd con621
cargo build --release
```

The binary outputs to `target/release/con621` (~2MB).

## Requirements

- Rust 1.56+ (2021 edition) to build
- **[ffmpeg](https://ffmpeg.org/)** — required only for **video** preview (`.webm`/`.mp4`). It is auto-downloaded on first use via [`ffmpeg-sidecar`](https://crates.io/crates/ffmpeg-sidecar) if not already on your system. Still images and GIFs work without it.
- A terminal with graphics support (Kitty, iTerm2, WezTerm, or Sixel) gives true inline images; any other terminal falls back to half-block rendering.
- Works on Windows, Linux, and macOS — no ncurses dependency.

## Usage

```
con621
```

(or `./target/release/con621` if built from source)

### Keybindings

#### Search Screen
| Key | Action |
|-----|--------|
| `Tab` | Cycle between fields |
| `Enter` | Execute search |
| `Esc` | Quit |

#### Results Screen
| Key | Action |
|-----|--------|
| `j` / `k` / `↑` / `↓` | Navigate posts |
| `Enter` | View post details |
| `i` | Toggle image/video preview |
| `o` | Open in browser |
| `d` | Download file |
| `n` / `p` | Next / previous page |
| `s` | Open settings |
| `q` / `Esc` | Back to search |

#### Detail Screen
| Key | Action |
|-----|--------|
| `j` / `k` | Scroll up / down |
| `h` / `l` / `←` / `→` | Previous / next post |
| `i` | Toggle image/video preview |
| `o` | Open in browser |
| `d` | Download file |
| `s` | Open settings |
| `q` / `Esc` | Back to results |

#### Settings Screen
| Key | Action |
|-----|--------|
| `k` / `+` / `↑` | Increase playback FPS |
| `j` / `-` / `↓` | Decrease playback FPS |
| `Enter` | Save |
| `Esc` | Back |

#### Global
| Key | Action |
|-----|--------|
| `?` | Toggle help |
| `Ctrl+C` | Force quit |

## Configuration

Settings are stored as JSON in your platform config directory, e.g. `~/.config/con621/config.json` on Linux:

```json
{
  "fps": 15
}
```

- `fps` — target frames-per-second for video/animation playback (1–60). Editable in the in-app Settings screen (`s`).

## License

Licensed under the [GNU General Public License v3.0 or later](LICENSE).
