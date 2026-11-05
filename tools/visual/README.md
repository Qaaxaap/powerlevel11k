# Visual render check

The render check runs `p11k` inside a real kitty terminal in a headless sway compositor and
compares the captured pixels with committed reference frames. It answers a different question
from the proxy tests: not "what bytes did the engine write", but "what did the user see".

The toolchain lives in the `visual` package, not in the default devShell: sway, kitty and grim
are heavy, and only this check needs them. It is also not part of `just verify`.

## Running

```sh
# Build the debug binary, capture the prompt in the four shells, compare the pixels.
nix develop --command just render-check

# After an intended rendering change: regenerate the reference frames, review every image,
# then commit the changed files under expected/.
nix develop --command just render-update
```

The four shells are captured in parallel, each with its own compositor, terminal and output
directory. Every shell prints one result line (`capture: zsh ok (7200ms)`); the full log of a run
stays in `OUT/<shell>/capture.log`. The capture takes about eight seconds; the comparison is a
few hundred milliseconds.

The captures and the diff images land in `.cache/render-check/`:

```text
.cache/render-check/
  zsh/p11k-zsh-000.png
  bash/p11k-bash-000.png
  fish/p11k-fish-000.png
  pwsh/p11k-pwsh-000.png
  diff-<shell>.png        # only for a failed comparison
```

## Reference frames

Each shell has its own reference frame at `tools/visual/expected/<system>/<shell>.png`. It is a
pixel-exact snapshot that a person reviewed: any pixel difference fails the check for that shell.
The shells paint the same header, but their input line, echo and redraw behaviour differ, so a
shared frame would fail for the shells that did not produce it. A screenshot of a correct prompt
is the baseline; there is no hand-written expected rendering.

Comparing decoded pixels matters: comparing PNG files would also compare the encoder and the
metadata, which do not affect the screen. `compare.sh` uses ImageMagick's AE metric and reports
the number of differing pixels.

References are per architecture (`expected/x86_64-linux/`, `expected/aarch64-linux/`) because
the font raster can differ between them.

## Determinism

Every input that can move a pixel is pinned:

- headless sway with one output at a fixed size, fixed XKB layout and locale;
- kitty from the same Nix closure, with `tools/visual/kitty.conf` pinning the family, size,
  cursor shape, cursor blink, padding and the terminal colours;
- Maple Mono NF as the font family, from the wrapper's `FONTCONFIG_FILE`;
- a fixed repository at `/tmp/p11k-visual-repo` (clean `main`) and a fixed HOME without user
  rc files;
- `tools/visual/reference.kdl`, which has no segment that changes on its own (time, execution
  time, background jobs) and no machine-dependent segment (the OS icon);
- the cursor is painted with the background colour (`kitty.conf`), so whether the terminal draws
  it, and in which blink phase, cannot enter the frame;
- the frame is captured after two identical grim captures, never after a fixed sleep.

Without `/dev/dri` the compositor falls back to pixman. A GPU run (gles2) can produce slightly
different pixels, so the reference frame belongs to the environment that produced it. Updating
kitty, the font or the renderer means regenerating the reference in the same commit.

## Adding a shell or a scene

`capture.sh` starts one independent run per shell in `--shells` (default: `zsh bash fish pwsh`)
and waits for all of them; the recorded frame name is `<out>/<shell>/p11k-<shell>-000.png`. A new
scene is a new theme file plus a new capture step; keep the reference set small and reviewable.

## CI

The `render` job in `.github/workflows/ci.yml` runs the check on x86_64. It does not run in
`just verify`: the visual toolchain devShell is heavy and the reference frames only exist for
x86_64. Like the other jobs it is skipped for documentation-only changes through the `changes`
job.

The job shares the x86_64 cache keys on purpose: both jobs need the same flake inputs and the
same cargo build directory, and a second key per job would store a second copy. The Nix store
cache limit is 3G to leave room for the visual devShell. A failed run uploads
`.cache/render-check/` (the captures and the diff images) as a workflow artifact.

