# p11k — English source messages.
#
# This file is the source of truth for every user-facing string. Message ids are stable:
# changing an id changes the code that looks it up, so rename them in both places.
# Entries with variables take them from the call site (see `i18n::t_args`).

# ===== command line and configuration =====
cli-cannot-read-config = cannot read config, falling back to the built-in lean theme: { $detail }
cli-unknown-preset = unknown preset, falling back to the built-in lean theme (expected lean/classic/rainbow/pure): { $preset }
cli-not-in-session-unset = not inside a p11k session (P11K_ENGINE_PID is unset)
cli-invalid-engine-pid = invalid P11K_ENGINE_PID (must be a positive process id)
cli-not-in-session-missing = not inside a p11k session (session data is missing; restart the session)
cli-cannot-write-reload-request = cannot write the reload request: { $detail }
cli-cannot-remove-reload-request = cannot remove the reload request: { $detail }
cli-cannot-signal-engine = cannot signal the engine (the session may have exited)
cli-cannot-read-session-file = cannot read the session file: { $detail }
cli-token-mismatch = the session token does not match; the pid may belong to another process
cli-no-theme-file = this session has no theme file: it was started with --preset or the built-in theme
cli-unknown-shell = unknown shell in the config, falling back to $SHELL (expected zsh/bash/fish/pwsh): { $shell }
config-invalid-hex-colour = { $key }: a hex colour needs six digits: { $value }
config-unknown-colour-name = { $key }: unknown colour name: { $name }; { $help }
config-colour-help = use a name, 0-255, #rrggbb or default
config-colour-index-out-of-range = { $key }: colour index is outside 0-255: { $index }
config-expected-colour = { $key }: expected a colour (0-255, #rrggbb, a colour name or default)
config-kdl-parse-error = KDL parse error: { $detail }
config-state-needs-name = `state` needs a name (a positional string)
config-element-needs-text = { $element } needs text (a positional string)
config-layout-required = the config needs a layout block
config-unknown-key = unknown config key: { $key }
config-mode-fallback = "{ $declared }" cannot be displayed by this locale; falling back to "{ $effective }"
config-glyph-fallback = { $field } "{ $glyph }": cannot be displayed; using the built-in default
config-unrenderable-glyphs = the config has a glyph this locale cannot display in mode "{ $mode }": { $glyphs }
config-cannot-render = cannot render the theme: { $error }

# ===== engine =====
engine-inner-shell-failed = the inner shell failed to start
engine-terminal-degraded = terminal type "{ $term }" has no usable terminfo entry; the prompt may not redraw correctly
main-recursive-launch = recursive launch detected (p11k started inside a p11k session)
main-bootstrap-hint = make sure the bootstrap line in your shell rc is guarded, e.g.:
main-cannot-exec-shell = cannot exec the shell: { $detail }

# ===== presets and wizard options =====
preset-lean = Lean.
preset-classic = Classic.
preset-rainbow = Rainbow.
preset-pure = Pure.
preset-original = Original.
preset-snazzy = Snazzy.
wizard-yes = Yes.
wizard-no = No.
wizard-restart = Restart from the beginning.
wizard-quit = Quit
wizard-font-diamond = Does this look like a diamond (rotated square)?
wizard-font-lock = Does this look like a lock?
wizard-font-angle = Does this look like >< but taller and fatter?
wizard-style-title = Prompt Style
wizard-shade-lightest = Lightest.
wizard-shade-light = Light.
wizard-shade-dark = Dark.
wizard-shade-darkest = Darkest.
wizard-colours-title = Prompt Colors
wizard-colours-256 = 256 colors.
wizard-colours-8 = 8 colors.
wizard-colour-title = Prompt Color
wizard-frame-colour-title = Frame Color
wizard-charset-title = Character Set
wizard-charset-unicode = Unicode.
wizard-charset-ascii = ASCII.
wizard-rprompt-title = Non-permanent content location
wizard-side-left = Left.
wizard-side-right = Right.
wizard-time-title = Show current time?
wizard-time-12h = 12-hour format.
wizard-time-24h = 24-hour format.
wizard-separators-title = Prompt Separators
wizard-separator-angled = Angled.
wizard-separator-vertical = Vertical.
wizard-heads-title = Prompt Heads
wizard-tails-title = Prompt Tails
wizard-shape-flat = Flat.
wizard-shape-blurred = Blurred.
wizard-shape-sharp = Sharp.
wizard-shape-slanted = Slanted.
wizard-shape-round = Round.
wizard-height-title = Prompt Height
wizard-height-one-line = One line.
wizard-height-two-lines = Two lines.
wizard-connection-title = Prompt Connection
wizard-connection-disconnected = Disconnected.
wizard-connection-dotted = Dotted.
wizard-connection-solid = Solid.
wizard-frame-title = Prompt Frame
wizard-frame-none = No frame.
wizard-frame-full = Full.
wizard-spacing-title = Prompt Spacing
wizard-spacing-compact = Compact.
wizard-spacing-sparse = Sparse.
wizard-icons-title = Icons
wizard-icons-few = Few icons.
wizard-icons-many = Many icons.
wizard-flow-title = Prompt Flow
wizard-flow-concise = Concise.
wizard-flow-fluent = Fluent.
wizard-transient-title = Enable Transient Prompt?
wizard-preview = Preview (current directory + exit code):

# ===== wizard chrome and writing the config =====
wizard-needs-terminal = p11k configure needs an interactive terminal
wizard-use-theme = Use this theme?
wizard-yes-write-config = Yes, write the config.
wizard-aborted = p11k configuration wizard has been aborted.
wizard-nothing-written = Nothing was written.
wizard-overwrite = p11k config file already exists. Overwrite { $path }?
wizard-new-config = New config: { $path }.
wizard-add-to-rc = Add one of these to your shell rc:
wizard-engine-is-theme = The engine is the theme: remove any ZSH_THEME/theme setup first, otherwise they will stack.
