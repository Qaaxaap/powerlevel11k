# p11k agent guide

p11k (powerlevel11k) is a prompt engine written in Rust. The engine runs as a long-lived process and proxies a pty. When a shell prints a prompt, the engine takes over and draws the header.

## Before you start

1. Read the commit conventions, the code style and the AI policy in `CONTRIBUTING.md`.
2. Enter the nix devShell. Run every build, test and lint there, not against the system tools of the same name.

   ```bash
   nix develop
   ```

3. Use the project recipes. `just check` is the fast gate. CI runs `just verify`.

   ```bash
   nix develop --command just verify
   ```

## Completion claims

- Include the command and its output in a completion claim. A claim without output evidence is not a completion.
- Write unverified parts in this format:

  ```text
  Unverified: <what>
  Reason: <missing environment, missing credentials, interactive operation>
  Local check: <command or steps>
  ```

- Add a test for each behaviour change. Check first that the test fails without the change.
- Treat an unstable test as a defect. Fix the cause. Do not hide it with retries.

## Do not

- Do not degrade silently. That covers `|| true`, `2>/dev/null`, a fallback when a font or a tool is missing, and a swallowed error. Let failures surface. If an error must be ignored, write the reason in a comment and record it in the pull request.
- Do not make unrelated refactors. Do not format whole files. Do not leave debug output or temporary files.
- Do not commit `.cache/`, `target/`, or agent directories for one tool (`.dsh/`, `.claude/`, `.cursor/`). The project agent assets are this file, `docs/glossary.md` and `.agents/skills/`.
- Do not change generated files (`docs/images/`) without saying so. Change a generated file only by running its generator.

## Decisions

- Ask the maintainer before a change to public behaviour, an interface, a dependency, the config language or the release process.
- Ask when a trade-off is unclear. Do not lower the specification or drop options by guessing.

## Text

- English is the working language of this repository. Write code, comments, commit messages, the interface and the documentation in English.
- The maintainer maintains the Chinese documentation and the Chinese interface in sync with the English text. Do not change the Chinese versions.
- Terminology follows `docs/glossary.md`.
- Comments explain causes, constraints, units and failure modes. Do not restate what the code already says.
- User-facing strings follow the gettext msgid style: lowercase start, no closing period, details after a colon.
- When the English text disagrees with the code's behaviour, the code wins. Point out the mismatch in the pull request.

## Pull request and commit text

- Cite only what the repository contains. Reviewers cannot see drafts in `.cache/`, scripts from a local skill, or a plan outside the repository.
- Put project commands in the verification evidence, such as `just verify` and `just spell`. Do not cite helper scripts that are not in the repository.
- Do not write internal plan numbers, phase names or task codes. A commit message follows the same rules.

## Interactive environment

- The engine depends on a real terminal: kitty, tmux, the user's zsh config. Mark behaviour you cannot reproduce locally as unverified. The maintainer accepts it.
- Run terminal experiments in a controlled session. Do not touch the user's terminal windows. Do not `pkill` the engine.

## Project skills

Project agent skills live in `.agents/skills/`. Each skill is a directory with a `SKILL.md` entry point. Harnesses that support the Agent Skills directory load these skills automatically. Other harnesses do not load them. Read the `SKILL.md` directly when a task matches.

## Routing

- Contribution process, code style, commit conventions, AI policy: `CONTRIBUTING.md`
- Config language: `docs/config-language.md`
- Terminology: `docs/glossary.md`
- Demo images: the Demo images section of `CONTRIBUTING.md`
