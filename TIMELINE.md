## UPDATED ON : 2026-10-20

### chore (2026-10-20) — Author rewrite, commit signing, and Prettier CI hook

1. **Author & Signing**: Rewrote history to attribute commits to Gaurav Saini <me@gauravsaini.com> and SSH-signed all commits for verified GitHub badge status.
2. **Prettier Setup**: Added root package.json, pnpm setup, .prettierrc, .prettierignore, Husky pre-commit hook, and lint-staged to run Prettier automatically on every commit.
3. **Tests**: All cargo tests passing (9 unit + 1 conformance + 5 CVE + 7 multigroup = 22 passed).
4. **Files changed**: .husky/pre-commit, .prettierrc, .prettierignore, package.json, pnpm-lock.yaml, .gitignore, and all JS/JSON/MD formatted files.

## UPDATED ON : 2026-10-21

### feat (2026-10-21) — CI/CD workflow and multi-platform npm publish pipeline

1. **GitHub Actions CI**: Added `.github/workflows/CI.yml` modeled after `ajv-napi` for multi-platform native builds (macOS x64/arm64, Windows x64, Linux x64/arm64 gnu/musl via Zig), automated testing, and provenance-enabled npm publish on `v*` tags.
2. **NAPI Package & Tests**: Added author metadata and publishConfig to `napi/package.json`, generated `binding.js` & `binding.d.ts`, and updated equivalence tests to cleanly resolve `braces` reference.
3. **Tests**: 4/4 napi test suites pass (multigroup, CVE, corpus artifacts, check-silence), 22/22 cargo tests pass.
4. **Files changed**: .github/workflows/CI.yml, napi/package.json, napi/binding.js, napi/binding.d.ts, napi/test-multigroup.js, napi/test-corpus-artifacts.js
