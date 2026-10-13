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
| `Ctrl+P` / `Ctrl+K` command palettes | 🚧 |
| Workspace files + boilerplate + in-app editor | 🚧 |
| nvim interoperability | 🚧 |
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

## nvim interoperability

The app hosts a small control API on `127.0.0.1:9527` and ships a nvim plugin
(`contrib/nvim/leetr.lua`) that talks to it:

| nvim command | Effect |
| --- | --- |
| `:LeetrOpen <slug>` | open a problem in the app |
| `:LeetrPull` | load the app's current buffer into nvim |
| `:LeetrPush` | send the nvim buffer to the app |
| `:LeetrRun` | run the sample tests |
| `:LeetrSubmit` | submit the current buffer |

Saving in nvim can auto-push, and the app reloads a workspace file when it changes on
disk — so edits flow both ways without any manual copying.

## License

MIT
