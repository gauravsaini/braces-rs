## UPDATED ON : 2026-10-20

### chore (2026-10-20) — Author rewrite, commit signing, and Prettier CI hook

1. **Author & Signing**: Rewrote history to attribute commits to Gaurav Saini <me@gauravsaini.com> and SSH-signed all commits for verified GitHub badge status.
2. **Prettier Setup**: Added root package.json, pnpm setup, .prettierrc, .prettierignore, Husky pre-commit hook, and lint-staged to run Prettier automatically on every commit.
3. **Tests**: All cargo tests passing (9 unit + 1 conformance + 5 CVE + 7 multigroup = 22 passed).
4. **Files changed**: .husky/pre-commit, .prettierrc, .prettierignore, package.json, pnpm-lock.yaml, .gitignore, and all JS/JSON/MD formatted files.
