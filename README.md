# Leetr

A **lightweight** desktop LeetCode client in Rust with [`egui`](https://github.com/emilk/egui)
— and first-class **nvim interoperability** so you never copy-paste between your editor and
a browser again.

![Browsing problems and reading a statement](docs/browse.png)

No embedded Chromium, no WebKit, no async runtime. One small native binary that reads
problems, runs your code and submits it, while **your own nvim** does the editing.

```
$ du -h target/release/leetr    13M
$ ldd target/release/leetr | wc -l
5
```

Measured rather than asserted, on a 1260×1052 window:

| metric | Leetr | blank `eframe` window (same renderer) |
| --- | --- | --- |
| launch → answering on the control API | **0.12 s** | — |
| proportional set size (PSS) | 54 MB | 49 MB |
| resident set size (RSS) | 166 MB | 160 MB |

So Leetr's own footprint is about **5 MB** on top of the toolkit floor. Plain RSS reads ~165 MB
because it counts shared driver pages that every GL process maps — PSS is the honest number, and
the difference is the GPU stack rather than this program.

---

## Features

| | |
| --- | --- |
| **Browse problems** | Virtualised list of 4000+ problems with difficulty, acceptance rate, solved/attempted markers, filters and paging |
| **Read statements** | LeetCode's HTML rendered as real rich text: inline code, monospace examples, bullets, collapsible hints and starter code |
| **Submit solutions** | Run sample tests and submit, with a verdict panel showing runtime, produced vs expected output and compile/runtime errors |
| **nvim interoperability** | Edit in your own nvim; Leetr and nvim share a file on disk plus a loopback control API, so nothing is ever copy-pasted |
| **Dark mode** | A curated dark theme (default) and a matching light theme, persisted in `config.toml` |
| **`Ctrl+P`** | Fuzzy problem selector — jump to any loaded problem by id, title or slug |
| **`Ctrl+K`** | Command palette — theme, reload, filters, links, quit |
| **Lightweight** | A **13 MB** release binary linking only `libc`, `libm` and `libgcc` — no GTK, no WebKit, no OpenSSL, no async runtime |

### Command palettes

`Ctrl+P` ranks by fuzzy match with bonuses for word boundaries and contiguous runs, so
`sum` puts **Two Sum** first. When nothing matches locally, an inline
**"Search LeetCode for …"** action falls back to a server-side search — no per-keystroke
network traffic.

![The Ctrl+P problem selector](docs/palette.png)

### Running and submitting

![An accepted submission](docs/judge.png)

---

## Install

```bash
git clone https://github.com/HimanshuSardana/leetr
cd leetr
cargo install --path .          # or: cargo build --release
```

Requirements: Rust 1.95+ (the `eframe`/`egui` 0.36 stack needs it), and on Linux the usual windowing headers
(`libxkbcommon`, `libxcb`, `libx11`, `libwayland`, `libgl`). Optional: `nvim`, `tmux`.

The first run writes default config to `~/.config/leetr/`.

## Credentials

Browsing problems and reading statements needs **nothing**. Running tests and submitting
needs a LeetCode session:

1. Log in to <https://leetcode.com>.
2. DevTools (F12) → **Application → Storage → Cookies → `https://leetcode.com`**.
3. Copy the `LEETCODE_SESSION` and `csrftoken` cookie values into
   `~/.config/leetr/credentials.toml`.
4. Restart Leetr — a **signed in** badge appears in the status bar.

Environment variables `LEETCODE_SESSION` and `LEETCODE_CSRFTOKEN` override the file.
`credentials.toml` lives outside the repository and is never committed.

```bash
leetr whoami    # verifies the session and reports how many problems you've solved
```

## Keybindings

| Shortcut | Action |
| --- | --- |
| `Ctrl+P` | Problem selector — fuzzy-jump to any loaded problem |
| `Ctrl+K` | Command palette — theme, reload, filters, links, quit |
| `Tab` | Switch from the statement to the code pane |
| `Ctrl+O` | Open the current solution in nvim |
| `Ctrl+'` | Run the sample tests |
| `Ctrl+Enter` | Submit the solution (respects the [safety rail](#safety-rail)) |
| `Ctrl+B` | Show / hide the problem sidebar |

While a palette is open it takes the keyboard instead:

| Shortcut | Action |
| --- | --- |
| `Up` / `Down`, `Tab` / `Shift+Tab` | Move the selection |
| `Enter` | Run the highlighted command / open the highlighted problem |
| `Esc` | Close the palette |

`Tab` switches panes on the main view but moves the selection inside a palette,
so it never does both at once. While the **problem filter** has focus, `Tab` is
left to the field instead, so typing a query and then pressing Tab does not yank
you into the code pane. (Fixing that was its own commit: it used to change 31,699
pixels of the view mid-query.)

### Deliberate shortcut overrides

The global shortcuts are checked before any widget sees the key, so a few of them
shadow a built-in text-editing binding. That is a choice, not an oversight:

* `Ctrl+K` — egui's text fields bind this to "delete to end of line". Leetr claims
  it for the command palette, which is the usual convention for a global palette
  and is far more useful here than kill-line in a single-line filter box.
* Everything else is free: egui's text fields only bind `Ctrl+H`, `Ctrl+K`,
  `Ctrl+U` and `Ctrl+W`, so `Ctrl+P`, `Ctrl+O`, `Ctrl+'`, `Ctrl+Enter`, `Ctrl+B`
  and `Tab` shadow nothing.

## CLI

The same client works headlessly:

```bash
leetr list --limit 20 --slug          # browse
leetr list --todo --search matrix     # filter (needs credentials)
leetr show two-sum                    # statement as readable terminal text
leetr show two-sum --raw              # raw statement HTML
leetr languages                       # submission languages
leetr whoami                          # verify the session
leetr install-nvim-plugin             # install the nvim plugin
```

## Editing: nvim interoperability

**A file on disk is the source of truth.**

```
~/.local/share/leetr/workspace/<id>-<slug>/
    solution.py     # both Leetr and nvim edit this
    problem.md      # the statement, readable offline
    meta.json       # ids / title / language
```

* Opening a problem creates the directory and seeds the solution from LeetCode's
  **boilerplate** for your configured language. An existing file is never overwritten.
* Saving in nvim updates Leetr: the app notices the file change and reloads.
* Editing in Leetr writes the file back, so `:e!` / `:LeetrPull` picks it up.
* **Open in nvim** launches your editor on the file. Inside tmux it opens a split,
  otherwise it uses `$TERMINAL`, otherwise plain `nvim`. Override with `editor_command`
  in `config.toml`.

### nvim plugin

```bash
leetr install-nvim-plugin        # writes ~/.config/nvim/lua/leetr.lua
```

Then add `require("leetr").setup()` to your nvim config.

| Command | Effect |
| --- | --- |
| `:LeetrStatus` | show what Leetr has open, plus the last verdict |
| `:LeetrOpen <slug>` | ask Leetr to open a problem |
| `:LeetrPull` | load Leetr's buffer into this window |
| `:LeetrPush` | send this buffer to Leetr |
| `:LeetrRun` | run the sample tests |
| `:LeetrSubmit` | submit, then report the verdict |
| `:LeetrResult` | show the last judge verdict |
| `:LeetrPort <n>` | point the plugin at another port |

Saving any `*/solution.*` file pushes it automatically, so the normal workflow is just
Leetr and nvim side by side.

### Control API

Leetr hosts a small API on `127.0.0.1:9527` (configurable) — loopback only:

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | liveness + version |
| `GET` | `/state` | problem, language, file, code, solved flag, judge result |
| `GET` | `/code` | current buffer as text |
| `POST` | `/code` | replace the current buffer |
| `POST` | `/open` | open a problem by slug |
| `POST` | `/run` | run the sample tests |
| `POST` | `/submit` | submit the buffer (respects the solved-guard) |

Reads are served from a snapshot the UI publishes, so they never block. Writes are queued
and applied on the next frame — and they *wake* the UI, so an idle window still reacts.

## Safety rail

Leetr refuses to submit to a problem that is **not already marked solved** on your account,
and the refusal happens *before* any request reaches LeetCode — an attempt can never be
spent accidentally:

```
refusing to submit to `median-of-two-sorted-arrays`: it is not in your solved set.
Leetr only submits to problems you have already solved —
set `allow_unsolved_submit = true` in config.toml to override.
```

The Submit button is disabled for unsolved problems too, and if the solved set has not
loaded yet Leetr refuses rather than guessing.

Setting `allow_unsolved_submit = true` in `config.toml` turns the guard off — and because
that is exactly the kind of flag that is easy to leave switched on, doing so is **never
silent**: every submission to an unsolved problem then carries a warning in the verdict
panel, in the status bar and in `/state`, naming the problem and the config key.

## Architecture

```
src/
  app.rs           eframe App: state, panels, background work
  bg.rs            threads + channel plumbing (no async runtime)
  config.rs        config.toml + credentials.toml
  theme.rs         dark/light palettes, difficulty colours
  glyphs.rs        every UI symbol, with a test that they exist in the default fonts
  editor.rs        workspaces, per-language extensions, buffers, mtime reloading
  nvim.rs          editor launch strategy + embedded nvim plugin + installer
  interop.rs       loopback control API (tiny_http)
  model.rs         domain types, decoupled from the API schema
  leetcode/
    graphql.rs     query strings + wire types
    client.rs      blocking HTTP, auth, problem list, solved set
    html.rs        statement HTML -> rich-text blocks
    submit.rs      judge protocol, verdicts, solved-guard
  ui/
    problem_list.rs  virtualised list, filters
    statement.rs     statement rendering
    code_editor.rs   editor pane + verdict panel
    palette.rs       fuzzy matching + Ctrl+K / Ctrl+P
    status_bar.rs
tests/live_api.rs  live integration tests (ignored by default)
contrib/nvim/      the nvim plugin, embedded into the binary
```

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test                       # unit tests; offline, no network
cargo test --test live_api -- --ignored   # hits the real LeetCode API
```

CI runs on every push and PR:

* `fmt`, `clippy -D warnings` and the offline test suite;
* an **`msrv` job** that reads `rust-version` straight out of `Cargo.toml` and builds with
  exactly that toolchain, so the declared minimum is verified rather than asserted.

Separately, a **nightly** workflow runs the live tests against the real LeetCode API. The
offline suite cannot notice that an undocumented endpoint changed, and that has already
happened twice here — the solved-status filter enum went from `SOLVED` to `AC`, and
`problemsetQuestionList` silently started capping `limit` at 100. The nightly run needs no
credentials; add `LEETCODE_SESSION` and `LEETCODE_CSRFTOKEN` as repository secrets to also
cover the authenticated solved-set lookup.

The live tests cover paging, search, statement fetching, the language list, the
authenticated solved-set lookup, and headless rendering of the real statement and problem
list panes. The judge client is tested against a mock judge over real HTTP (headers, CSRF,
referer, bodies, polling) **plus fixtures copied verbatim from live responses** — an
earlier hand-written fixture missed that `code_output` is an array for sample runs, which
only the real API caught. The judge now also has **live** coverage: sample runs
(`interpret_solution`) execute code without creating a submission, so the verdict path is
exercised against the real judge without spending an attempt. That is what surfaced the
`Accepted`-for-wrong-code bug described below.

## Notes and limitations

* **No Monaco / embedded browser.** The original idea was a Monaco editor, but an embedded
  WebKit/Chromium is ~50MB+ of runtime dependencies and fights "lightweight". The file +
  nvim + control API design gives the same "edit in a real editor, submit from here"
  workflow with none of the weight.
* Problem pages load lazily; the palette searches what is loaded, then falls back to a
  server-side search.
* The judge is polled every 700ms for up to 30s, with a short retry on the rate limiting
  LeetCode applies when you press Run several times quickly.
* **Link targets are dropped, their text is kept.** Measured before deciding: 44 problems
  contained 5 `<a href>`s between them, all incidental references to Wikipedia or cplusplus.com
  whose link text already carries the meaning ("in-place", "Run-length encoding"). Carrying a URL
  on every inline span would touch the whole rendering path for almost no reader benefit, so this
  is a deliberate "no" rather than an oversight.
* **Statement images are referenced, not rendered** — fetching and decoding them would mean an
  image loader and a decoder, which this app deliberately avoids. The URL is shown so a diagram
  is never silently missing (several problems, e.g. `add-two-numbers`, carry one the prose
  refers to).
* Verified on Linux (X11 and Wayland). The macOS/Windows launch paths are implemented but
  untested.

## License

MIT — see [LICENSE](LICENSE).
