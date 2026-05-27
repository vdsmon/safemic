## Context

SafeMic is a Rust + native AppKit tray app, single binary, macOS-only, currently distributed as an `rcodesign` self-signed DMG via GitHub Releases. Local release build is driven by `mise run release`, which chains `cargo build --release --target aarch64-apple-darwin` → `cargo bundle` → `rcodesign sign` → `hdiutil create`. Versioning is via `Cargo.toml` `package.version`. No Apple Developer ID, no notarization.

Today's install UX: download DMG → mount → drag to `/Applications` → run `xattr -dr com.apple.quarantine` (or Open Anyway dance). Discoverable only if you read the README. No upgrade story beyond "do it again."

Stakeholders: end users (want one-line install), the maintainer (wants tag → release to be hands-off), and the Homebrew tap repo as a delivery channel.

## Goals / Non-Goals

**Goals:**

- `brew install --cask vdsmon/tap/safemic` installs and launches on a clean macOS arm64 box, no manual Gatekeeper workaround.
- Cutting a release is one command: `git tag v0.5.2 && git push --tags`. CI does the rest.
- Cask url + sha256 stay in sync with the DMG automatically; the maintainer reviews the auto-PR to the tap and merges.
- `mise run release` and CI release use the same recipe — no parallel-truth drift.

**Non-Goals:**

- Apple Developer ID + notarization (separate future change; unblocks upstream `homebrew-cask` submission).
- Submitting to upstream `homebrew/homebrew-cask`.
- Universal binary or x86_64 support (separate change; widens audience but complicates CI).
- In-app auto-update (Sparkle, etc.). Homebrew upgrade is the update story.
- Code signing the DMG itself (only the `.app` bundle is signed inside the DMG today).

## Decisions

### Decision 1: Cask, not Formula

**Choice**: Ship as a Homebrew Cask (binary `.app` from a DMG), not a Formula (build from source).

**Why**: SafeMic is a GUI app for end users. A Formula would force every install to compile Rust, install `cargo-bundle`, and set up `rcodesign`. Casks are the standard channel for prebuilt macOS GUI apps. Brew itself prompts `brew install --cask` for any URL-distributed app.

**Alternative considered**: Formula + manual bundle script. Rejected — multiplies install time by ~5 minutes per user and breaks for anyone without a working Rust toolchain.

### Decision 2: Personal tap, not upstream homebrew-cask

**Choice**: Host the cask in `github.com/vdsmon/homebrew-tap` (a separate repo), not in `homebrew/homebrew-cask`.

**Why**: Upstream homebrew-cask rejects apps that aren't Apple-notarized. SafeMic uses `rcodesign` self-signing, which is not Apple-notarized. Submission would bounce on the first review pass. A personal tap has zero gatekeeping — `brew tap <user>/<repo>` works immediately.

**Alternative considered**: Wait until notarization is set up, then submit upstream first. Rejected — blocks distribution on an Apple Developer subscription and a notarization pipeline, both larger than this change.

**Follow-up**: After notarization lands (separate change), submitting to upstream is additive — the personal tap can keep working in parallel.

### Decision 3: aarch64-only, gated by `depends_on arch: :arm64`

**Choice**: Cask explicitly requires arm64. Intel users get a clean refusal from Homebrew.

**Why**: The local release recipe is `--target aarch64-apple-darwin`; producing a universal binary would require a second `x86_64-apple-darwin` toolchain in CI, plus `lipo`, plus testing on an Intel runner the maintainer doesn't own. Out of scope for v1.

**Alternative considered**: Build universal binary in CI. Rejected for scope. Easy follow-up later — just add an x86_64 build step and `lipo`, then drop the `depends_on arch` line.

### Decision 4: Tag-triggered workflow, not branch-push

**Choice**: Release workflow trigger is `on: push: tags: ['v*']` only. Branch pushes and PRs do not invoke it.

**Why**: Avoids accidental releases. Avoids surprise tap PRs from feature work. Makes "tagging" the single human action that means "ship this."

**Alternative considered**: Manual `workflow_dispatch` trigger. Rejected — adds an extra click and an extra failure mode (forgetting to tag the right SHA after dispatching).

### Decision 5: Tap update via PR, not direct push

**Choice**: Workflow opens a PR to the tap repo. Merging the PR is a human action.

**Why**: The PR is the last sanity check on the rendered cask before users get it. If the workflow ever produces a bad sha256 (e.g., partial DMG upload), the maintainer catches it on PR review instead of after `brew install` failures hit users.

**Alternative considered**: Direct push to tap `main`. Rejected — one safety net is cheap, and the maintainer is the only consumer of the tap PR queue.

**Alternative considered**: `brew bump-cask-pr` via Homebrew's livecheck bot. Rejected — that bot only runs against casks already in upstream homebrew-cask, so it can't service a personal tap.

### Decision 6: `TAP_REPO_TOKEN` is a fine-grained PAT, not classic

**Choice**: Authentication to the tap repo uses a fine-grained PAT, scoped to `vdsmon/homebrew-tap` only, with `contents: write` + `pull-requests: write` permissions.

**Why**: Default `GITHUB_TOKEN` is scoped to the current repo and can't push to a different repo. A classic PAT works but has broader implicit scope. Fine-grained limits blast radius if the token leaks.

**Threat model**: A leaked token lets the attacker open a PR to the tap (not auto-merge it — maintainer review still required). The maintainer is the only reviewer; they would reject an unexpected bump. Worst case: a convincing-looking PR slips through review and ships malicious DMG via cask postflight. Mitigation depends on maintainer vigilance during PR review, plus the postflight only running `xattr` (no arbitrary script).

### Decision 7: `postflight` runs `xattr -dr com.apple.quarantine`

**Choice**: The cask's `postflight` block strips the quarantine xattr from `/Applications/SafeMic.app` after install.

**Why**: The DMG is not Apple-notarized, so macOS attaches `com.apple.quarantine` to the extracted `.app`. Launching from Finder triggers Gatekeeper, which refuses to open it ("damaged / cannot be opened"). Removing the xattr at install time means first launch works. This is a documented Homebrew pattern for self-signed apps.

**Alternative considered**: Tell users to `xattr -dr` themselves. Rejected — defeats the whole point of using brew.

**Alternative considered**: Get the user to manually approve in System Settings. Rejected — bad UX, scary first-run experience.

**Risk**: Stripping quarantine is a security-relevant action. Mitigated by: the user opted into our tap, the DMG sha256 is verified by brew before we run `xattr`, and `xattr` only affects the specific bundle path we installed. We are not stripping system-wide.

## Risks / Trade-offs

[Cargo version drifts from git tag] → The workflow's first step is a `Cargo.toml` version check against the tag string. Mismatch fails the build before any artifact is produced. Documented in the cargo-version-mismatch scenario.

[CI runner runs out of disk during `cargo clean && cargo build --release`] → `macos-latest` has ~14 GB free, release build needs ~3 GB. Comfortable margin; not an active risk but worth noting if dependencies bloat.

[Tap PR auto-merge gets misconfigured] → The PR is deliberately not auto-merged. Maintainer reviews and merges manually. If we later add `gh pr merge --auto`, that becomes a new risk vector; out of scope here.

[`TAP_REPO_TOKEN` leaks] → Fine-grained scope limits blast radius to the tap repo only. Leaking it does not grant push to this repo. Maintainer rotates token on suspicion. PR review at the tap is the second line of defense before users see the bad cask.

[`rcodesign sign` fails on a fresh CI runner] → `rcodesign` is installed via `cargo install apple-codesign` in the existing `mise run release:deps` task. CI runs the same path. Failure here would also break local builds, so existing dev process catches it.

[GitHub Releases asset URL changes format] → The cask uses the `https://github.com/vdsmon/safemic/releases/download/<tag>/<filename>` pattern, which has been stable for years. Low risk; if it changes, fix the template and re-cut a release.

[Postflight `xattr` removes quarantine for a sha256-verified-but-still-malicious DMG] → If the maintainer's repo is compromised and a bad DMG is uploaded with a matching cask sha256, users get a malicious app with quarantine stripped. This is the same threat model as the current manual `xattr` step the README tells users to run. The mitigation lives at GitHub auth + maintainer 2FA, not at the cask layer.

[User installs via cask, then a Cargo.toml version bump happens but no tag is pushed] → No release, no cask bump, no harm. Cask continues to point to the previous tagged version. Working as intended.

## Migration Plan

This is additive; no migration of existing installs.

For users on the manual DMG path:

1. They can ignore the new path and keep using DMGs.
2. They can migrate at any time: drag-uninstall the old `/Applications/SafeMic.app`, then `brew tap vdsmon/tap && brew install --cask safemic`. Settings at `~/Library/Application Support/mic-mute/settings.json` are preserved across this migration since both paths use the same bundle id and write path.

Rollback (for the maintainer, if cask distribution proves bad):

1. Delete or unpublish the cask in `vdsmon/homebrew-tap`. Existing installs keep working (the local `/Applications/SafeMic.app` is unaffected). New `brew install` calls fail with "cask not found." Manual DMG install remains available.
2. Optionally remove the GitHub Release artifacts to also break the direct-download path. Not recommended unless the artifacts are actively dangerous.

## Open Questions

- Should `Cargo.toml`'s `osx_minimum_system_version = "10"` be corrected to a realistic floor (e.g., macOS 13) as part of this change, since the cask metadata propagates it to users? Not strictly blocking; can ship with `"10"` and fix in a follow-up.
- Should the cask declare a `livecheck` block now, even though we don't use upstream Homebrew's bot? It's free metadata that documents the version source. Defer to implementation taste; non-blocking.
