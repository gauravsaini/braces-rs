## UPDATED ON : 2026-10-20

### chore (2026-10-20) — Author rewrite, commit signing, and Prettier CI hook

1. **Author & Signing**: Rewrote history to attribute commits to Gaurav Saini <me@gauravsaini.com> and SSH-signed all commits for verified GitHub badge status.
2. **Prettier Setup**: Added root package.json, pnpm setup, .prettierrc, .prettierignore, Husky pre-commit hook, and lint-staged to run Prettier automatically on every commit.
3. **Tests**: All cargo tests passing (9 unit + 1 conformance + 5 CVE + 7 multigroup = 22 passed).
4. **Files changed**: .husky/pre-commit, .prettierrc, .prettierignore, package.json, pnpm-lock.yaml, .gitignore, and all JS/JSON/MD formatted files.

## UPDATED ON : 2026-10-21

### feat (2026-10-21) — CI/CD workflow mirroring ajv-napi and npm publish pipeline

1. **GitHub Actions CI**: Mirrored `ajv-napi` CI-CD workflow (`.github/workflows/CI.yml`) with exact matrix (macOS x64/arm64, Windows x64/arm64), binding test suites (`test-macOS-windows-binding`), and automated provenance-enabled publish job on `v*` tags.
2. **NAPI Package & Tests**: Added author metadata and publishConfig to `napi/package.json`, added root `build` and `test` scripts, generated `binding.js` & `binding.d.ts`, and updated equivalence tests.
3. **Tests**: All 7 CI matrix & test jobs pass 100% green on GitHub Actions, 22/22 cargo tests pass.
4. **Files changed**: .github/workflows/CI.yml, package.json, napi/package.json, napi/binding.js, napi/binding.d.ts, napi/test-multigroup.js, napi/test-corpus-artifacts.js
