<!--
Keep this description complete before marking the pull request ready for review.
The Summary, Motivation, Type of Change, Testing and Checklist headings are
required; the other sections may be deleted, and so may any line under Type of
Change that does not apply.

An explanation does not replace a required check. If a required statement is
not true yet, keep the pull request as a draft.
-->

## Summary

<!-- What changed and why, in one or two sentences? -->

## Motivation

<!-- What problem does this solve? -->

## Type of Change

<!-- Mark all that apply; keep only the lines that apply. -->

- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Refactoring
- [ ] Build / packaging
- [ ] Documentation
- [ ] Translation only

## Related Issue

<!-- Example: Closes #123 -->

## Testing

<!--
List the commands you ran and what you saw. Where they apply:

  nix develop
  cargo fmt --all
  cargo clippy --all-targets --locked -- -D warnings
  cargo test --all --locked

Say why a command was not run rather than leaving this empty.
-->

## Manual Coverage

<!-- Mark what applies; delete the section if none of it does. -->

- [ ] Tested in zsh
- [ ] Tested in bash
- [ ] Tested in fish
- [ ] Tested in pwsh
- [ ] Tested the segments or states the change touches
- [ ] Tested at more than one terminal width
- [ ] Tested under a non-UTF-8 locale (ascii icon fallback)

## Checklist

<!--
Check every item; each one already covers the case where it does not apply.
English is the source language of both the interface and the documentation: a
contributor updates the English text, and the maintainer syncs the Chinese.
-->

- [ ] This PR is ready for review, or it is marked as a draft.
- [ ] I read and followed the relevant guidance in `CONTRIBUTING.md`.
- [ ] The change is one logical unit, with no unrelated work bundled in.
- [ ] I ran `cargo fmt --all`, or this PR changes no Rust code.
- [ ] I ran `cargo clippy --all-targets --locked -- -D warnings` and
      `cargo test --all --locked`, or this PR changes no Rust code.
- [ ] I ran `tools/i18n.sh extract` and `tools/i18n.sh update` if this PR
      changed user-facing English text; the Chinese wording is the maintainer's
      to fill in.
- [ ] A translation into a language other than English or Chinese is its own,
      translation-only PR with no code changes in it, or this PR adds none.
- [ ] I updated `docs/config-language.md` for config changes, or this PR does
      not change the config language.
- [ ] I updated `README.md`, or this PR changes no user-visible behaviour,
      installation or configuration.
- [ ] I used the canonical names for config keys, segments and icons.
- [ ] I self-reviewed the change and checked for new warnings or errors.

## Additional Notes

<!-- Follow-up notes, reviewer context, or known limitations. -->
