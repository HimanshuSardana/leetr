# Leetr

A **lightweight** desktop LeetCode client written in Rust with
[`egui`](https://github.com/emilk/egui) — and first-class **nvim interoperability**
so you never have to copy-paste between your editor and the browser again.

```
┌ Leetr ────────────────────────────────────────────────────┐
│ Problems            │ Statement                           │
│ ▸ 1. Two Sum        │                                     │
│   2. Add Two Numbers│  Given an array of integers ...      │
│   3. Longest Sub... │                                     │
└─────────────────────┴─────────────────────────────────────┘
```

## Why

The LeetCode web UI is heavy, and the usual workflow is: read the problem in a browser,
write the solution in nvim, then paste it back into a web form. Leetr removes the browser
and the paste step:

- browse problems and read full statements in a small native window,
- solve in **your** nvim (or the built-in lightweight editor),
- push/pull code between nvim and Leetr over a local control API,
- run sample tests and submit — from either side.

No embedded Chromium/WebKit. The whole thing is one small native binary.

## Status

Under active construction. Everything is developed in small, reviewable feature PRs.

| Feature | Status |
| --- | --- |
| App shell (egui/eframe) | ✅ |
| Config + credentials handling | ✅ |
| Dark mode | ✅ |
| LeetCode API client + CLI | ✅ |
| Problem list + statement viewer | 🚧 |
| `Ctrl+P` / `Ctrl+K` command palettes | ✅ |
| Workspace files + boilerplate + in-app editor | ✅ |
| nvim interoperability | ✅ |
| Run sample tests + submit | 🚧 |

## Requirements

- Rust 1.82+
- Linux (X11 or Wayland), macOS, or Windows
- Optional: `nvim` for the editor integration

## Build & run

```bash
cargo run --release
```

The first run writes default config files:

```
~/.config/leetr/config.toml
~/.config/leetr/credentials.toml   # see below
```

Solution workspaces live in `~/.local/share/leetr/workspace/`.

## CLI

The same API client is available headlessly, which is handy on its own and for
scripting from nvim:

```bash
leetr list --limit 20 --slug          # browse problems
leetr list --todo --search matrix     # filter (needs credentials)
leetr list --solved --limit 100       # only what you've solved
leetr show two-sum                    # print the statement as plain text
leetr show two-sum --raw              # print the raw statement HTML
leetr languages                       # supported submission languages
leetr whoami                          # verify the configured session
```

`leetr show` renders LeetCode's HTML statement into readable terminal text
(decoded entities, `10^4` superscripts, monospaced examples, bulleted
constraints) — the same renderer the GUI uses.

## Keybindings

| Shortcut | Action |
| --- | --- |
| `Ctrl+P` | Problem selector — fuzzy-jump to any loaded problem |
| `Ctrl+K` | Command palette — theme, reload, filters, links, quit |
| `Up` / `Down`, `Tab` / `Shift+Tab` | Move the palette selection |
| `Enter` | Run the highlighted command / open the highlighted problem |
| `Esc` | Close the palette |

Both palettes fuzzy-match on the label *and* the hint, so `theme`, `reload`,
`filters` or `quit` all find the right command. They rank word-boundary and
contiguous matches highest, which is why `sum` puts **Two Sum** first.

If `Ctrl+P` finds nothing locally, an inline **"Search LeetCode for …"** action
falls back to a server-side search — no per-keystroke network requests.

## Credentials

Browsing problems and reading statements works **without** any credentials. Running tests
and submitting solutions requires a LeetCode session:

1. Log in to <https://leetcode.com>.
2. DevTools (F12) → **Application → Storage → Cookies → `https://leetcode.com`**.
3. Copy the `LEETCODE_SESSION` and `csrftoken` cookie values into
   `~/.config/leetr/credentials.toml`.
4. Restart Leetr — a **signed in** badge appears in the status bar.

Environment variables `LEETCODE_SESSION` and `LEETCODE_CSRFTOKEN` override the file.

> `credentials.toml` lives outside the repository and is never committed.

## Safety rail

Submitting a solution to a problem that is **not already marked solved** on your account is
refused by default. Flip `allow_unsolved_submit = true` in `config.toml` if you want to
disable the guard.

## Editing: nvim interoperability

Leetr treats a **file on disk as the source of truth**, so your editor and the app are
never out of sync:

```
~/.local/share/leetr/workspace/<id>-<slug>/
    solution.py     # both Leetr and nvim edit this
    problem.md      # the statement, readable offline
    meta.json       # ids/title/language
```

* Saving in nvim updates Leetr (the app notices the file change and reloads).
* Editing in Leetr writes the file back, so `:e!` / `:LeetrPull` picks it up.
* **Open in nvim** in the Code tab launches your editor on the file — inside tmux it opens
  a split; otherwise it uses `$TERMINAL`; otherwise plain `nvim`. Configure
  `editor_command` in `config.toml` to override.

### Control API

The app hosts a small API on `127.0.0.1:9527` (configurable), used by the bundled plugin
and by anything else you want to script:

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | liveness + version |
| `GET` | `/state` | open problem, language, file, code, solved flag |
| `GET` | `/code` | the current buffer as text |
| `POST` | `/code` | replace the current buffer |
| `POST` | `/open` | open a problem by slug |

```bash
curl -s 127.0.0.1:9527/state
curl -sS -X POST -H 'Content-Type: application/json' \
     -d '{"slug":"two-sum"}' 127.0.0.1:9527/open
```

### nvim plugin

```bash
leetr install-nvim-plugin     # writes ~/.config/nvim/lua/leetr.lua
```

Then add `require("leetr").setup()` to your nvim config. Commands:

| nvim command | Effect |
| --- | --- |
| `:LeetrStatus` | show what Leetr has open |
| `:LeetrOpen <slug>` | ask Leetr to open a problem |
| `:LeetrPull` | load Leetr's buffer into this window |
| `:LeetrPush` | send this buffer to Leetr |
| `:LeetrPort <n>` | point the plugin at another port |

Saving a file matching `*/solution.*` pushes it automatically, so the normal flow is just
`leetr` + `nvim` side by side with no copying at all.

## License

MIT
