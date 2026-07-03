<p align="center">
  <img width="128" src="./assets/icons/128x128@2x.png" style="padding:0.5rem;">
</p>

<h1 align="center">SafeMic for macOS</h1>

SafeMic is a system-wide microphone mute for macOS. Toggle every input device at once with a global shortcut or from the menu bar, and a bezel styled like the system volume OSD confirms every change.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./snapshots/popup/muted-dark.png">
    <img width="150" src="./snapshots/popup/muted-light.png" alt="Mute bezel showing a red slashed microphone icon">
  </picture>
</p>

Mute with <kbd>Cmd</kbd> <kbd>Shift</kbd> <kbd>M</kbd> (configurable) or from the menu bar dropdown, which has **Mute**, **Settings...**, **About**, and **Quit**. The menu bar icon always shows the current state, even when something else changes the mute state behind SafeMic's back.

## Features

- Mutes every CoreAudio input device through the native mute control. Devices without one get their input volume pulled to zero instead (and restored on unmute), and devices with neither control are skipped.
- Polls for hot-plugged devices (USB, Bluetooth) and re-asserts mute on anything that appears while the mic should be off.
- Detects devices that accept a mute command but never actually change state (some virtual devices, Microsoft Teams Audio for example) and stops managing them, so one lying driver can't make SafeMic report the wrong state for your real microphones.
- Global shortcut with click-to-record capture in Settings and validation against macOS-reserved combos.
- Confirmation bezel styled like the system volume OSD: red slashed mic when muted, appears on whichever monitor your cursor is on, ignores mouse events, and stays out of screenshots and recordings.
- Menu bar icon adapts to the menu bar's light/dark appearance.
- Launch at login.
- The app follows the system appearance, or you can force Light or Dark in Settings.

## Settings

Open **Settings...** from the menu bar icon. Every change applies and saves as you make it, with no Save button. The window edits the app appearance (System, Light, or Dark), the mute shortcut (click the shortcut chip to record a new one), launch at login, and how long the confirmation bezel stays on screen.

The same settings live in `~/Library/Application Support/safemic/settings.json`, which you can also edit by hand, and SafeMic picks up file changes while running. Missing keys fall back to defaults, so the file may be partial.

```json
{
  "mic_shortcut": {
    "modifiers": ["shift", "meta"],
    "key": "M"
  },
  "launch_at_login": false,
  "popup_duration_ms": 1000,
  "theme": "system"
}
```

| Key | Type | Default | Description |
|---|---|---|---|
| `mic_shortcut.modifiers` | `string[]` | `["shift", "meta"]` | Any of `"shift"`, `"meta"` (Cmd), `"ctrl"`, `"alt"` (Option). |
| `mic_shortcut.key` | `string` | `"M"` | Single key identifier: `"A"`-`"Z"`, `"0"`-`"9"`, `"F1"`-`"F20"`, `"Space"`, etc. |
| `launch_at_login` | `bool` | `false` | Start the app automatically on login. |
| `popup_duration_ms` | `number` | `1000` | How long the confirmation bezel stays visible after a mute/unmute event, in milliseconds. `0` hides the bezel entirely, though the menu bar icon still updates. |
| `theme` | `string` | `"system"` | App appearance: `"system"`, `"light"`, or `"dark"`. Affects SafeMic's windows and the bezel. The menu bar icon always follows the OS appearance. |

## Limitations

SafeMic is best-effort, **not** a hardware privacy switch.

- Only mutes devices CoreAudio can control. Devices with neither a mute nor an input volume control are skipped.
- Devices that misreport their mute state get excluded rather than controlled.
- Polling can leave brief mute gaps on newly connected devices.
- Drivers can lie. If you need real assurance, use a hardware mute switch, unplug the mic, or revoke microphone permission in macOS.

## Install

### Homebrew (recommended)

```sh
brew tap vdsmon/tap
brew install --cask safemic
```

Apple Silicon (arm64) only for now. The cask strips the macOS quarantine attribute on install so the app launches without a Gatekeeper warning.

To upgrade later: `brew upgrade --cask safemic`. To uninstall completely (including settings): `brew uninstall --cask --zap safemic`.

### Manual install

I have not elected to sign the app by joining the Apple Developer Program. The releases are self-signed and can be installed by bypassing the typical app security on macOS, or by building and bundling the app yourself with the instructions further down this README.

[View releases](https://github.com/vdsmon/safemic/releases)

After downloading the DMG and dragging `SafeMic.app` to `/Applications`, run:

```sh
xattr -dr com.apple.quarantine "/Applications/SafeMic.app"
```

(or use the "Open Anyway" button under System Settings > Privacy & Security).

### Permissions

On first launch, grant SafeMic **Microphone** access (CoreAudio control) and **Input Monitoring** (global shortcut) when prompted. The global shortcut also needs **Accessibility**, which macOS never prompts for, so add SafeMic manually under System Settings > Privacy & Security > Accessibility.

## Build

Install [mise](https://mise.jdx.dev/). It manages the Rust toolchain plus the dev dependencies (watchexec, lefthook) and runs the project tasks.

Install build deps + bundle the app:

```sh
mise run build
```

A finder window opens to the bundle at `./target/aarch64-apple-darwin/release/bundle/osx`.

## Develop

### Setup

Install [mise](https://mise.jdx.dev/), then:

```sh
mise run init
```

### Run

Run and watch for changes:

```sh
mise run start
```

### Build

```sh
mise run build
```

<details>
<summary>Release</summary>
Create a certificate to self-sign.

```sh
openssl req -x509 -newkey rsa:2048 -keyout sign.key -out sign.crt \
    -days 3650 -nodes -subj "/CN=safemic"
cat sign.key >> sign.crt
rm sign.key
```

Build a release.

```sh
mise run release
```

</details>

## Acknowledgements

SafeMic is a rebrand and continuation of [mic-mute](https://github.com/brettinternet/mic-mute) by Brett Gardiner, released under the MIT License. The original copyright is preserved in `LICENSE`.
