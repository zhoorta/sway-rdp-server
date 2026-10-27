# Security policy

Waydesk is early software and has not been independently reviewed. The threat model and the known open items are in [docs/THREAT-MODEL.md](docs/THREAT-MODEL.md);
the internal review passes are in [docs/SECURITY-REVIEW.md](docs/SECURITY-REVIEW.md).

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private report: **Security → Report a vulnerability** on
<https://github.com/zhoorta/waydesk/security/advisories/new>.

Include the version (`waydesk-server --version`), the distribution, and the steps to reproduce. You will get an answer within a few days; this is a small project, so
fixes are made as quickly as one person can, and credit is given if you want it.

## Scope

In scope: the daemon, the root helper, authentication and session start, the units and their sandboxing, the packages and the apt repository (including its signing key).
Out of scope: drive, printer and smartcard redirection, RD Gateway, and running the RDP port exposed to the internet (the documentation says not to).

## Supported versions

Only the latest release.
