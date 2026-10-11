# Changelog

Notable changes, newest first. Generated from
[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) by
`just changelog`.

## Unreleased

### Bug fixes
- **engine:** Open one blank row per prompt-add-newline
- **engine:** Repaint the prompt whenever the shell reprints it
- **engine:** Use the 7-bit form of cursor save and restore
- **engine:** Cover the prompt zsh reprints while the input line is idle

### CI
- Build the flake checks in CI, with tmux and terminfo

### Documentation
- Add recorded demo images and a demo-first README (#6)
- Add an empty all-contributors slot

### Refactoring
- **engine:** Hand the placeholder to the shell instead of substituting it

### Tests
- Run the bash tests on an interactive bash
- Run the proxy tests inside tmux instead of a bare pty

## 1.0.0-alpha.1 — 2026-09-12

### Chores
- Drop the unimplemented benchmark skeleton
- Switch to a prerelease version scheme

### Documentation
- Rewrite the README for users rather than developers
- Split CONTRIBUTING by language and bring it up to date
- Drop the performance contract
- Add a pull request template
- Bring the English README in line with the Chinese one
- Complete the license, add issue templates and a CI badge
- Drop a reference to a file that is not in the repository

### Features
- **engine:** Read the theme from the XDG config directory
- **engine:** Name the proxied shell in the theme config

### Translations
- Rewrite the wording that reads as speech

## 0.1.0 — 2026-09-12

### Bug fixes
- Align SafePrint docs and protocol version with official binaries
- Terminate probe paths with NUL
- Align empty-repo branch and skip-index stats with original
- Claim protocol version v1.5.5 to match p10k master
- Rebuild dir fd chains and path prefixes in shard scan
- Honor negative count caps and cache the index tree
- Expand context user/host in-segment and escape % in zsh substitution
- **zsh-theme:** Compute gap width with sed instead of zsh patterns
- **zsh-theme:** Kubecontext content expansion and cache splitting
- **zsh-theme:** Escape % in ssh template substitution
- **zsh:** Make gitstatus client actually work (rewrite IPC + framing)
- **zsh-theme:** Align default colors with p10k lean (236 bg scheme)
- **zsh-theme:** Unset __p11k_right instead of setting to empty
- **zsh-theme:** Vi_mode colors and no icon, matching p10k
- **zsh-theme:** Hide vi_mode segment outside vi mode
- **zsh-theme:** Measure width with p10k's prompt-expansion bisection
- **zsh-theme:** Gap margin and instant flag residue
- **zsh:** Pending never clears when req_id is overwritten while in flight
- **zsh:** Always redraw prompt on gitstatus response
- **zsh-theme:** Format command execution time like p10k
- **zsh-theme:** Use hourglass icon for command execution time
- **zsh-theme:** Render prompt boundary arrows with foreground color
- **zsh-theme:** Add p10k segment whitespace around icons and content
- **zsh:** Store gitstatus daemon pgid as integer so stop kills it
- **daemon:** Close directory fds after scan (DirStack RAII)
- **daemon:** Exit when parent zsh dies (PR_SET_PDEATHSIG)
- **zsh:** Exec daemon and pass -p so it tracks zsh, not a grandchild
- **zsh:** Disown the gitstatus daemon so kitty ignores it on close
- **zsh:** Drop PR_SET_PDEATHSIG from daemon since disown rebinds its parent
- **zsh:** Orphanize the daemon out of zsh's process tree
- **zsh-theme:** Emit OSC 133 prompt markers for kitty
- **engine:** Handle POLLHUP, preserve $? in precmd, support external zshrc
- **engine:** Align zle geometry with real terminal via placeholder PROMPT
- **engine:** Git counts uncapped to match p10k (-1 limits)
- **engine:** Forward terminal pixel size to inner pty
- **engine:** Redraw prompt window on resize (TRAPWINCH + SIGWINCH)
- **engine:** Resize redraw keeps visible history (no \e[2J)
- **engine:** Clear P11K_ENGINE when the real prompt is ready
- **engine:** Bash/fish resize + fish _p11k_winch init
- **theme:** Clear header rows before redraw (no stale right-aligned text)
- **engine:** Fish resize via pending_redraw (engine-driven)
- **engine:** Fish resize clears screen (scroll breaks \e[2A)
- **engine:** Fish resize via fish_prompt COLUMNS detection
- **engine:** Reorder announce before passthrough (fish resize aa race)
- **engine:** Resize redraw waits for placeholder+buffer (cursor fix)
- **engine:** Resize only redraws header, no prompt interference
- **engine:** Resize defers prompt redraw (zsh cursor + restore ❯)
- **render:** No duplicate prompt_char, config-driven resize/vcs redraw, segment spacing
- **render:** Every segment gets a default background (incl. text), unify separator semantics
- **render:** Redraw header(vcs) on prompt-ready if git result arrived earlier
- **render:** Don't redraw prompt on zsh continuation lines (PS2)
- **render:** Subtract frame prefix+suffix from right-align budget (no wrap)
- **config:** Merge same-name segment nodes (os icon=... + foreground=... two lines)
- **render:** Hide empty segments entirely (no orphan icon, e.g. background_jobs=0)
- **render:** Right sub-separator uses segment fg on bg (p10k style), visible on black
- **render:** Right-start uses right prompt bg color (revert overreach)
- **render:** Right-segment triangle fg=current bg, bg=previous bg (mirror of left)
- **config:** Report prompt_char width errors via clean-shell drop-in
- **render:** Unicode display width for wide chars (emoji/CJK)
- **render:** Right-column segments put icon last (p10k 镜像)
- **render:** Text-middle stays between icon and content on right column
- **render:** Version segments fall back to stderr output
- **render:** Terraform_version reads only first line
- **render:** Nix_shell only matches pure/impure (p10k 对齐)
- **engine:** Vi_mode initial INSERT and keymap normalization (zsh vi)
- **engine:** Wizard style preview indent pushed the right column off-screen
- **gitstatus:** Submodules (gitlink) reported dirty — dir stat vs 160000 mode always mismatched
- **gitstatus:** Staged count stale after git add — cache keyed only by HEAD
- **gitstatus:** HEAD read once per repo — commit/checkout invisible to staged cache
- **render:** Segment prefix/suffix were parsed but never drawn
- **wizard:** Few icons still showed the remote vcs icon
- **render:** Icon trailing space doubled with segment padding
- **render:** Content-less segments still drew their icon
- **engine:** Transient prompt ignored the config when the env var leaked
- **config:** Format to_kdl output — autoformat + quoted strings
- **render:** Vcs/status state colors were parsed but never applied
- **render:** Vcs icon color and bare spaces inside segments
- **render:** Right-column spacing — icon gap on the wrong side, no gap after separators
- **render:** Exec-time hourglass icon, vcs untracked/conflicted colors
- **render:** Match p10k's vcs count glyphs, order and action word
- **render:** Vcs tag and detached-HEAD rendering, meta color
- **gitstatus:** Remote fields need a valid remote and a tracking ref
- **render:** Drop the right column when the row cannot fit
- **engine:** No longer clears the screen when the real prompt arrives
- **render:** Right column keeps zsh's RPROMPT indent
- **wizard:** Keep frame glyphs transparent, colour segments per style
- **zsh:** Stop overriding completion colours
- **pwsh:** Redraw the prompt when the window shrinks
- **pwsh:** Don't mistake terminal replies for user typing
- **gitstatus:** Collapse nested ifs flagged by clippy 1.97
- **engine:** Report a failed inner-shell spawn, and dump the log on test timeout
- **engine:** Drop the PSModulePath/PSHOME pwsh sniffing
- **wizard:** Make Ctrl-C, Esc and Ctrl-D actually quit
- **pwsh:** Report the command number, not the history count
- **config:** Stop warning about segment behavior properties
- **render:** Match p10k's icon rendering
- **packaging:** Install the catalogs and stop baking a build path
- **engine:** Stop panicking when stdout is a closed pipe

### Build
- Scaffold cargo workspace
- Scaffold p11k-gitstatus protocol and daemon crate
- Scaffold p11k-d binary entry
- Add git2 dependency for the repo object model
- **i18n:** Extract msgids with xgettext and guard the catalogs
- Add a flake with a pinned devShell, and run CI inside it
- Set rust-version to the real minimum (1.95, not 1.85)
- Let nix flake check actually run the tests

### CI
- Run fmt, clippy and tests on every push
- Cache the nix store between runs
- Bump actions/checkout to v7
- Check that the nix package still builds
- Build release artifacts on version tags
- Declare libgcc-s1 in the deb dependencies

### Chores
- Initialize repository with license, README and gitignore
- Lockfile update (add kdl dep for engine)
- Remove transient debug logs from git-refresh fix
- Drop dead pre-KDL render code
- **engine:** Tidy comments, drop stale and redundant notes
- **engine:** Strip redundant parenthetical asides in comments
- Drop comments from the CI workflow and Cargo.toml
- Anglicize test, bench and dev-facing messages
- Ignore nix build output
- Tighten the Chinese comments before review
- **i18n:** Refresh the translation template

### Documentation
- Add contributing guidelines
- Clarify AI involvement in README
- Document theme usage and current status
- Document instant prompt setup in README
- Reposition p11k as p10k theme + Rust kernel; archive zsh theme
- Add third-party notice for the vendored p10k theme
- Point main README at the engine line, reference compat branch
- Refresh kernel-roadmap wording, add gitstatus perf numbers
- KDL v2 naming conventions (kebab-case, fg/bg, #true) in README
- Point CONTRIBUTING at the config language spec
- Add KDL v2 config language spec
- **engine:** Adjust and simplify the wording of the comments
- Rewrite README for the engine's current state; split out zh copy
- Split config-language into en/zh copies; sync stale points
- Sync zh/en config language docs
- Document the new dir, vcs and segment surface
- Document layout right-indent
- Refresh the status/roadmap section
- Rewrite the comments in tests/proxy.rs
- Drop the stale PowerShell detection note
- Document installing from the flake
- Describe the release artifacts

### Features
- Implement gitstatus-compatible argument parsing
- Validate -G version glob with fnmatch-compatible matcher
- Implement gitstatus wire protocol codec
- Implement daemon main loop and liveness checks
- Implement repo object model via git2
- Wire repo cache into daemon request handling
- Implement index tree building and dirty detection
- Implement workdir scan with untracked cache pruning
- Implement untracked cache filesystem probe
- Implement index stats pipeline in repo
- Add p11k zsh theme MVP
- Extend theme with 30 segments, dir shortening, transient prompt
- Add system/version segments with caching and prompt % escaping
- Enrich vcs segment, format exec time, harden gitstatus client
- Add custom segments and double-source guard
- **zsh-theme:** Add 13 segments from user config coverage
- **zsh-theme:** Add instant prompt (p10k-compatible)
- **zsh:** Pass through p10k VCS limits and thread count to daemon
- **zsh-theme:** Rewrite prompt layout to match p10k multiline
- Vendor powerlevel10k theme and turn p11k.zsh-theme into a thin loader
- **engine:** M0 — pty host, transparent proxy, prompt-window theme
- **engine:** Exec bootstrap + recursion guard + fallback to ~/.zshrc
- **engine:** Vcs segment via in-process p11k-gitstatus API
- **engine:** Async git status via worker thread (p10k-style)
- **engine:** Instant header (p10k-style instant prompt)
- **engine:** Shell detection via feature env vars (zsh/bash/fish)
- **engine:** Bash support (rcfile injection + placeholder match)
- **engine:** Detect shell from --shell arg (env vars not exported)
- **engine:** Fish support (XDG_CONFIG_HOME + fish_prompt)
- **config:** KDL-v2 typed theme config (knus->kdl, #true booleans)
- **config:** Layout as block of boolean segment entries (dir #true …)
- **config:** Layout as ordered line nodes, no newline marker
- **render:** KDL-config-driven pure-text multi-line header renderer
- **render:** Wire KDL config into engine header rendering (--config)
- **render:** Powerline background blocks and segment separator arrow
- **render:** Configurable separators (segment/sub/end), correct powerline semantics
- **render:** Line text literals with env-var expansion
- **render:** Dir truncation to last N levels (config shorten-dir-length)
- **render:** Dir truncate_to_unique with per-level mtime cache
- **render:** Dir per-component coloring (anchor/shortened/normal), full path prefix
- **render:** Multi-line frame (first/newline prefix-suffix) for header rows
- **render:** Config-driven input prefix + dynamic placeholder width
- **render:** Segment icons, time segment, gap fill, right-side separator (A-group)
- **render:** Vcs branch icon, command_execution_time, background_jobs
- **render:** Vcs icon by remote domain (github/gitlab/bitbucket/aur, configurable)
- **render:** Os icon by distro (uname + /etc/os-release, aligned with p10k)
- **config:** Unescape \uXXXX in string values (icon/separators/frame overrides)
- **render:** Right-side same/异色 separator decision (right-segment)
- **config:** Frame pieces take colors
- **config:** Prompt_char char attribute + ERROR state
- **config:** Equal-width prompt_char glyphs across states
- **config:** Per-segment text-left/middle/right attach slots
- **render:** Context/user/host/root_indicator/date segments
- **render:** Env indicator segments (ssh/proxy/file-managers/…)
- **render:** Toolchain version segments (go/rust/node/php/java/dotnet/swift/terraform) + cpu_arch
- **render:** Env-manager segments (virtualenv/pyenv/nodeenv/rbenv/goenv/jenv/…)
- **render:** System-resource segments (load/ram/swap/disk_usage/battery)
- **render:** Cloud segments (aws/azure/gcloud/kubecontext/terraform)
- **render:** Network segments (ip/vpn_ip/wifi/public_ip)
- **render:** Misc complement segments (detect_virt/toolbox/dir_writable/asdf/…)
- **render:** Cloud-sub/framework/todo segments (google_app_cred/rspec/todo/…)
- **engine:** History and vi_mode segments via shell protocol
- **engine:** Prompt-add-newline loose layout (blank line between prompts)
- **engine:** Transient prompt (fold header on command submit) + main-loop reorder
- **engine:** Transient prompt (clear header on carriage return in passthrough)
- **engine:** Multi-line placeholder — shell prompt geometry counts header rows
- **engine:** Transient prompt (zsh) — fold header on command submit
- **engine:** Icon mode — nerdfont/compatible/ascii icon sets (POWERLEVEL9K_MODE)
- **engine:** Top-level icon{} table — per-mode icon overrides by icon name
- **engine:** When the terminal is not in UTF-8, revert to ascii
- **engine:** Built-in preset themes (lean/classic/rainbow/pure) + --preset
- **config:** Serialize Config back to KDL (round-trip)
- **engine:** Tail separators + 12h time format
- **engine:** P11k configure wizard — p10k question flow on Config structs
- **engine:** Wizard per-option live prompt previews (p10k style)
- **render:** Per-segment whitespace padding, matching p10k
- **config:** Gap-foreground for the gap filler (p10k GAP_FOREGROUND)
- **config:** Sub-foreground / right-sub-foreground (p10k sep_color)
- **engine:** Full dir shortening strategies (p10k SHORTEN_STRATEGY)
- **engine:** Exec precision and p10k duration format
- **engine:** Vcs branch shortening + configurable count symbols
- **presets:** Default vcs branch shortening (>32 → 12…12)
- **engine:** Vcs changeset (p10k SHOW_CHANGESET)
- **engine:** Home-folder-abbreviation + prompt-add-newline count
- **engine:** Prompt_char vi-mode states + configurable vi_mode text
- **engine:** Dir path-separator-foreground
- **render:** Vcs wip marker, remote branch, push counts and dash
- **render:** Dir shortening only when the row overflows
- **render:** SHOW_RULER ruler line above the header
- **render:** Dir show-writable swaps in the lock icon
- **config:** DIR_CLASSES directory classes
- **render:** Symfony2_version and symfony2_tests segments
- **wizard:** Templates carry the global background, like p10k
- **i18n:** Translate user-facing text with gettext
- **shell:** Add pwsh support
- **cli:** Add --version and --help
- **config:** Warn about keys the engine does not recognise
- **status:** Implement p10k's extended status states
- **vcs:** Implement p10k's VCS_DISABLED_WORKDIR_PATTERN
- **engine:** Add `p11k reload`
- **i18n:** Look for the catalogs beside the executable too

### Other
- Scaffold dirty-detection benchmark
- Drop compat-only theme, engine line starts here
- Relicense to LGPL-3.0-or-later

### Performance
- Reuse index tree structure when only stat fields change
- Use raw libgit2 diff with notify and progress early-stop
- **gitstatus:** Parallel dirty scan, root-fd path stat (320ms→98ms)

### Refactoring
- Make parse_request return Request directly
- Drop the ineffective git_index object cache
- Classify dirty candidates without libgit2 diff
- English comments everywhere, drop dead default-config
- **engine:** Placeholder-prompt geometry, orphaned inner shell, OSC 133
- **config:** Lean theme enables segments by presence (niri-style flags)
- **config:** Accept only documented KDL spellings
- Resolve all clippy/build warnings
- **presets:** Build Config structs with parameterized colors
- **render:** Drop a bg fallback that can never run

### Tests
- Scaffold protocol and compat test stubs
- Add daemon integration tests
- Add index tree, dirty detection and scan tests
- Add real-repo end-to-end daemon test
- Add differential tests against official gitstatusd
- **render:** Init segment in struct literal (clippy)

### Translations
- Phrase the help footer formally, and point zh at README.zh.md
- Phrase the help environment section formally
- Anglicize the embedded shell template comments
- Translate the remaining Chinese comments to English

