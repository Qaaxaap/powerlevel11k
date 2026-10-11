# Security policy

The project is in alpha: only the latest release and `main` are supported, and fixes ship in
the next tag rather than as backports.

## Reporting a vulnerability

Use GitHub's [private vulnerability reporting](https://github.com/Qaaxaap/powerlevel11k/security/advisories/new).
Please do not open a public issue for a security problem.

A useful report says which version (`p11k --version`), which shell and terminal, what an
attacker gains, and how to reproduce it. You can expect an answer within a week.

## Where the risk is

The engine owns a pty, runs a shell under it, and writes a temp state directory with an
announce file and an ack file that the inner shell reads and writes. Reports about that
directory, those files, the bytes passed through, or the `p11k-d` wire protocol are the most
valuable ones. Every `unsafe` block carries a justification comment; a report that shows one
of them to be unsound is welcome.
