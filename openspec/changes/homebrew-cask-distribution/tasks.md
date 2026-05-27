## 1. Cask template

- [x] 1.1 Create `packaging/homebrew/safemic.rb.tmpl` with placeholders `{{version}}`, `{{sha256}}`, `{{url}}`; stanzas: `version`, `sha256`, `url`, `name "SafeMic"`, `desc`, `homepage "https://github.com/vdsmon/safemic"`, `depends_on arch: :arm64`, `app "SafeMic.app"`
- [x] 1.2 Add `postflight do; system_command "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "#{appdir}/SafeMic.app"]; end`
- [x] 1.3 Add `zap trash:` list covering `~/Library/Application Support/mic-mute`, `~/Library/LaunchAgents/com.vdsmon.safemic.plist`, `~/Library/Caches/com.vdsmon.safemic`
- [x] 1.4 Render template locally with current `Cargo.toml` version and a placeholder sha256, run `brew audit --cask --new --strict /path/to/rendered.rb` to surface any cask-syntax errors before the workflow ships it; iterate on the template until clean

## 2. Release workflow

- [x] 2.1 Create `.github/workflows/release.yaml` with trigger `on: push: tags: ['v*']` and a single job `release` on `runs-on: macos-latest`, `permissions: { contents: write }`
- [x] 2.2 Job step: checkout the tagged commit (`actions/checkout@v4` with `fetch-depth: 0`)
- [x] 2.3 Job step: install mise via `jdx/mise-action@v2`; run `mise install` so the toolchain matches `mise.toml`
- [x] 2.4 Job step: cargo version check — parse `Cargo.toml` `package.version`, compare against `${GITHUB_REF_NAME#v}`; fail the job with a clear error if they differ
- [x] 2.5 Job step: stage signing cert — write `${{ secrets.RCODESIGN_CERT_PEM }}` to `sign.crt`, mode `0600`, in the workspace root so `mise run release` finds it where the local recipe expects
- [x] 2.6 Job step: run `mise run release` (will compile + bundle + rcodesign + hdiutil → `safemic-<version>-aarch64-apple-darwin.dmg` in repo root)
- [x] 2.7 Job step: compute sha256 — `shasum -a 256 safemic-*.dmg > safemic-*.dmg.sha256`
- [x] 2.8 Job step: publish GitHub Release with `softprops/action-gh-release@v2`, upload both DMG and `.sha256`, `draft: false`, `prerelease: false`, `fail_on_unmatched_files: true`

## 3. Tap repo bump

- [x] 3.1 Job step: render `packaging/homebrew/safemic.rb.tmpl` to `/tmp/safemic.rb` substituting `{{version}}`, `{{sha256}}` (from the file we just produced), and `{{url}}` = `https://github.com/vdsmon/safemic/releases/download/${GITHUB_REF_NAME}/safemic-${VERSION}-aarch64-apple-darwin.dmg`
- [x] 3.2 Job step: checkout `vdsmon/homebrew-tap` via `actions/checkout@v4` with `token: ${{ secrets.TAP_REPO_TOKEN }}`, `path: tap`
- [x] 3.3 Job step: copy rendered file to `tap/Casks/safemic.rb`, create branch `bump-safemic-${VERSION}`, commit with message `safemic ${VERSION}`, force-push the branch
- [x] 3.4 Job step: open a PR with `gh pr create` (or reuse if one exists for the same branch) titled `safemic ${VERSION}`, body linking to the GitHub Release; auth via `GH_TOKEN=${{ secrets.TAP_REPO_TOKEN }}`

## 4. Tap repo bootstrap (one-time human step)

- [ ] 4.1 Create `github.com/vdsmon/homebrew-tap` (empty public repo, no template)
- [ ] 4.2 Locally render `packaging/homebrew/safemic.rb.tmpl` with the most recent existing release (or cut `v0.5.2` first and use its values) and commit as `Casks/safemic.rb`
- [ ] 4.3 Add a one-paragraph `README.md` to the tap: install instructions (`brew tap vdsmon/tap && brew install --cask safemic`) and a note that the cask is auto-bumped from `vdsmon/safemic`
- [ ] 4.4 In the tap repo, configure branch protection on `main`: require PR review + status checks pass before merge (so the workflow-opened PRs go through a deliberate human merge)

## 5. Secrets + permissions

- [ ] 5.1 Create a fine-grained PAT scoped to `vdsmon/homebrew-tap` only, with `Contents: Read and write` + `Pull requests: Read and write`
- [ ] 5.2 Add it as `TAP_REPO_TOKEN` under `vdsmon/safemic` → Settings → Secrets and variables → Actions
- [ ] 5.3 Move the contents of the local `sign.crt` (the rcodesign self-sign cert generated per README) into repo secret `RCODESIGN_CERT_PEM` so step 2.5 has a value
- [ ] 5.4 Verify `sign.crt` is in `.gitignore` (it already should be, but confirm before pushing)

## 6. Documentation

- [x] 6.1 Edit `README.md`: add an `## Install` section above any existing manual install instructions with `brew tap vdsmon/tap` and `brew install --cask safemic` as the primary path; demote existing DMG instructions to `### Manual install` underneath
- [x] 6.2 Edit `CLAUDE.md`: in the build/dev commands table, add a `Release` row pointing at the tag push trigger; add a one-paragraph note under that table explaining that tagging `vX.Y.Z` is the canonical release action and that `mise run release` is the dev-time mirror of CI
- [x] 6.3 Note in `CLAUDE.md` that the cask repo lives at `github.com/vdsmon/homebrew-tap` (for future-Claude context)

## 7. Verification on a tagged release

- [ ] 7.1 Bump `Cargo.toml` `package.version` to `0.5.2` in a regular commit on `main`; do not tag yet
- [ ] 7.2 Tag `v0.5.2` on that commit and push the tag; confirm the release workflow starts
- [ ] 7.3 Confirm the GitHub Release for `v0.5.2` exists with the DMG + `.sha256` attached and the sha256 file's hash matches `shasum -a 256` of the DMG
- [ ] 7.4 Confirm a PR titled `safemic 0.5.2` lands in `vdsmon/homebrew-tap`; review the rendered `Casks/safemic.rb` and merge
- [ ] 7.5 On a clean arm64 macOS shell, run `brew tap vdsmon/tap && brew install --cask safemic`; confirm `/Applications/SafeMic.app` exists, opens from Finder without a Gatekeeper dialog, and the tray icon appears
- [ ] 7.6 Run `brew uninstall --cask --zap safemic`; confirm `/Applications/SafeMic.app` is gone and `~/Library/Application Support/mic-mute/` is gone
- [ ] 7.7 Re-install via `brew install --cask vdsmon/tap/safemic` to confirm the round trip
