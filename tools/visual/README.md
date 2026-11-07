# Visual render check

The render check runs `p11k` inside a real kitty terminal in a headless sway compositor and
compares the captured pixels with committed reference frames. It answers a different question
from the proxy tests: not "what bytes did the engine write", but "what did the user see".

The toolchain lives in the `visual` package, not in the default devShell: sway, kitty and grim
are heavy, and only this check needs them. It is also not part of `just verify`.

## Running

```sh
# Capture 16 frames (4 shells × 3 widths + dirty scene) and compare them.
nix develop --command just render-check [-j N]

# After an intended rendering change: capture new frames into expected-pending/.
nix develop --command just render-update [-j N]

# After reviewing every pending image against the intended layout:
nix develop --command just render-accept
```

`-j/--jobs N` sets the bats parallelism (default 4). `just render-update` never touches
`expected/`; it writes to `tools/visual/expected-pending/<arch>/`, which is ignored by git.
Running `just render-accept` copies the reviewed pending images into `tools/visual/expected/`
and removes the pending directory. A frame becomes a reference only after that review.

The four shells are four bats test files under `tools/visual/tests`; each test captures one
frame and prints one TAP result (`ok 1 bash reference frame at 700x450`). A failed test shows the
capture log, and the full log of every run stays in `.cache/render-check/<frame>/capture.log`.
With `-j 4` the 16 frames take about 25 seconds; the comparison is a few hundred
milliseconds.

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

The clean scene runs in a fixed repository at `/tmp/p11k-visual-repo` (clean `main`). The dirty
scene runs in `/tmp/p11k-visual-repo-dirty`, which has one stash, one staged change, one
unstaged change and one untracked file. Both repositories, the fixed HOME and the launcher
scripts are created once by `prepare.sh` before the parallel tests start.

Each frame is a pixel-exact snapshot that a person reviewed: any pixel difference fails the
check. The shells paint the same header, but their input line, echo and redraw behaviour differ,
so a shared frame would fail for the shells that did not produce it. A screenshot of a correct
prompt is the baseline; there is no hand-written expected rendering.

Comparing decoded pixels matters: comparing PNG files would also compare the encoder and the
metadata, which do not affect the screen. `compare.sh` uses ImageMagick's AE metric and reports
the number of differing pixels.

References are per architecture (`expected/x86_64-linux/`, `expected/aarch64-linux/`) because
the font raster can differ between them.

## Determinism

Every input that can move a pixel is pinned:

- headless sway with one output at the fixed frame size, fixed XKB layout and locale;
- kitty from the same Nix closure, with `tools/visual/kitty.conf` pinning the family, size,
  cursor shape, cursor blink, padding and the terminal colours;
- Maple Mono NF as the font family, from the wrapper's `FONTCONFIG_FILE`;
- the two fixed repositories above and a fixed HOME without user rc files;
- `tools/visual/reference.kdl`, which has no segment that changes on its own (time, execution
  time, background jobs) and no machine-dependent segment (the OS icon);
- the cursor is painted with the background colour (`kitty.conf`), so whether the terminal draws
  it, and in which blink phase, cannot enter the frame;
- the frame is captured after two identical grim captures, never after a fixed sleep; the
  helper waits `--min-wait 1` for zsh/bash/fish and 2 seconds for pwsh (its startup is slower),
  plus `--type-wait 1` after typing the command, so the input echo is complete before the
  stability check starts;
- `record-tui.sh` pins `WLR_RENDERER=pixman`: the software renderer is deterministic and works
  without a GPU, so a local run and CI produce the same pixels. A GPU (gles2) frame can differ by
  a pixel, so gles2 is only for manual experiments and its frames are not references.

Updating kitty, the font or the renderer means regenerating the references in the same commit.

## Adding a shell or a scene

Each shell is one bats file in `tools/visual/tests`; one test captures one frame through
`helper.bash` (`record-tui.sh` + `compare.sh`). `prepare.sh` creates the shared repositories,
HOME and launcher scripts once, before the tests run in parallel. A new scene is a new theme or
repository state in `prepare.sh` plus one test per shell; keep the reference set small and
reviewable.

## CI

The `render` job in `.github/workflows/ci.yml` runs the check on x86_64. It does not run in
`just verify`: the visual toolchain devShell is heavy and the reference frames only exist for
x86_64. Like the other jobs it is skipped for documentation-only changes through the `changes`
job.

The job shares the x86_64 cache keys on purpose: both jobs need the same flake inputs and the
same cargo build directory, and a second key per job would store a second copy. The Nix store
cache limit is 3G to leave room for the visual devShell. A failed run uploads
`.cache/render-check/` (the captures and the diff images) as a workflow artifact.
