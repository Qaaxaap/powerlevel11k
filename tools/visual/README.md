# Visual render check

[中文](README.zh.md)

The render check runs `p11k` in a real kitty terminal. The terminal runs in a headless sway
compositor. The check compares the captured pixels with committed reference frames. The proxy
tests answer a different question. They ask "what bytes did the engine write". This check asks
"what did the user see".

The toolchain lives in the `visual` package. The default devShell does not include it. Sway,
kitty and grim are heavy. Only this check needs them. The check is also not part of
`just verify`.

## Running

```sh
# Capture 16 frames (4 shells × 3 widths + dirty scene) and compare them.
nix develop --command just render-check [-j N]

# After an intended rendering change: capture new frames into expected-pending/.
nix develop --command just render-update [-j N]

# After a person reviews each pending image against the intended layout:
nix develop --command just render-accept
```

`-j/--jobs N` sets the bats parallelism. The default is 4. `just render-update` never touches
`expected/`. It writes to `tools/visual/expected-pending/<arch>/`, and git ignores this
directory. `just render-accept` copies the reviewed pending images into
`tools/visual/expected/` and removes the pending directory. A frame becomes a reference only
after that review.

The four shells are four bats test files under `tools/visual/tests`. Each test captures one
frame and prints one TAP result (`ok 1 bash reference frame at 700x450`). A failed test shows
the capture log. The full log of each run stays in
`.cache/render-check/<frame>/capture.log`. With `-j 4`, the 16 frames take about 25 seconds.
The comparison takes a few hundred milliseconds.

The captures and the diff images land in `.cache/render-check/`:

```text
.cache/render-check/
  zsh-700x450/p11k-zsh-700x450-000.png
  zsh-560x450/p11k-zsh-560x450-000.png
  zsh-900x450/p11k-zsh-900x450-000.png
  zsh-dirty/p11k-zsh-dirty-000.png
  diff-<frame>.png        # only for a failed comparison
```

## Frames

Each shell has four frames:

| frame | width | scene |
| --- | --- | --- |
| `<shell>-700x450.png` | 700 px | clean repository (`reference`) |
| `<shell>-560x450.png` | 560 px | clean repository (narrow) |
| `<shell>-900x450.png` | 900 px | clean repository (wide) |
| `<shell>-dirty.png` | 800 px | repository with a stash and dirty counts |

The clean scene runs in a fixed repository at `/tmp/p11k-visual-repo`. The repository has a
clean `main` branch. The dirty scene runs in `/tmp/p11k-visual-repo-dirty`. This repository has
one stash, one staged change, one unstaged change and one untracked file. `prepare.sh` creates
both repositories, the fixed HOME and the launcher scripts once. This step occurs before the
parallel tests start.

Each frame is a snapshot that a person reviewed. The comparison ignores antialiased pixels and
color differences of 1% or less. A difference in layout, text, color or position is larger than
that and fails the check. The shells paint the same header. Their input line, echo and redraw
behaviour differ. A shared frame would therefore fail for the shells that did not produce it. A
screenshot of a correct prompt is the baseline. There is no hand-written expected rendering.

The check compares decoded pixels. A PNG file comparison would also compare the encoder and the
metadata. Those items do not affect the screen. `compare.sh` runs `odiff --aa --threshold 0.01`.
The comparison ignores antialiased pixels and small color differences. It reports the number of
different pixels.

References are per architecture (`expected/x86_64-linux/`, `expected/aarch64-linux/`). The font
raster can differ between architectures.

## Determinism

The check pins each input that can move a pixel:

- A headless sway with one output at the fixed frame size. The XKB layout and the locale are
  fixed.
- Kitty from the same Nix closure. `tools/visual/kitty.conf` pins the family, the size, the
  cursor shape, the cursor blink, the padding and the terminal colours.
- Maple Mono NF as the font family. The wrapper provides it through `FONTCONFIG_FILE`.
- The two fixed repositories above and a fixed HOME. The HOME has no user rc files.
- `tools/visual/reference.kdl`. No segment changes on its own (time, execution time, background
  jobs). No segment depends on the machine (the OS icon).
- `kitty.conf` paints the cursor with the background colour. The frame therefore does not
  depend on whether the terminal draws the cursor or on the blink phase.
- The check captures the frame after two identical grim captures. It never uses a fixed sleep.
  The helper waits `--min-wait 0.5` for zsh, bash and fish. It waits `--min-wait 1.5` for pwsh
  because pwsh starts slower. The helper also waits `--type-wait 0.2` after the command
  finishes. The input echo is then complete before the stability check starts.
- `record-tui.sh` pins `WLR_RENDERER=pixman`. The software renderer is deterministic and does
  not need a GPU. A local run and a CI run therefore produce the same pixels. A GPU (gles2)
  frame can differ by one pixel. Use gles2 only for manual experiments, and do not use its
  frames as references.

A change to kitty, the font or the renderer requires new references in the same commit.

## Adding a shell or a scene

Each shell has one bats file in `tools/visual/tests`. One test captures one frame through
`helper.bash`. `helper.bash` calls `record-tui.sh` and `compare.sh`. `prepare.sh` creates the
shared repositories, the HOME and the launcher scripts once. This step occurs before the tests
run in parallel. A new scene needs a new theme or a new repository state in `prepare.sh`, plus
one test for each shell. Keep the reference set small and easy to review.

## CI

The `render` job in `.github/workflows/ci.yml` runs the check on x86_64. The job does not run in
`just verify`. The visual toolchain devShell is heavy, and the reference frames exist only for
x86_64. The `changes` job skips this job for documentation-only changes. The other jobs use the
same rule.

The job shares the x86_64 cache keys on purpose. Both jobs need the same flake inputs and the
same cargo build directory. A second key for each job would store a second copy. The Nix store
cache limit is 3G. This limit leaves room for the visual devShell. A failed run uploads
`.cache/render-check/` (the captures and the diff images) as a workflow artifact.
