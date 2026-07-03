# Native-adaptive appearance with semantic red

The settings window was locked to dark appearance and the About window was a borderless card chasing a branded design mock (`target.png` in the about-preview gate), both painted with hardcoded sRGB samples. We decided all windows follow the system appearance (light and dark) using semantic NSColors and native AppKit controls, with red demoted from brand accent to semantic red: it appears only on the muted state, warnings, and the app icon, while interactive controls take the user's system accent color. Rationale: the app should read as a first-party macOS utility, and hardcoded palettes were the root cause of the dark-lock (CALayer colors don't adapt), so going semantic removes a whole class of appearance bugs.

## Considered Options

- Dark-locked branded look, polish only — rejected: ignores light-mode users, keeps the CALayer color debt.
- Native materials with brand-red accent on controls — rejected: reads as a themed app, and tinting native controls red fights the system accent everywhere.

## Consequences

- The About design mock is retired; the about-preview gate flips from mock-conformance to self-regression (new targets captured from the accepted build, dark + light).
- Settings snapshot baselines are re-captured in both appearances; the `status_ok` preview state is gone because auto-apply success is now silent (error-only feedback), replaced by `status_err`.
- Custom-drawn surfaces must use appearance-adaptive vehicles (NSBox, NSVisualEffectView, template images, semantic colors), not raw CALayer color assignments.
