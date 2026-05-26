<p align="center">
  <img width="128" src="./assets/icons/128x128@2x.png" style="padding:0.5rem;">
</p>

<h1 align="center">Mic Mute for macOS</h1>

A system-wide mute for macOS microphones with a global shortcut and visual confirmation of mute status. Inspired by [VCM](https://learn.microsoft.com/en-us/windows/powertoys/video-conference-mute) for Windows.

![popup window screenshot indicating the microphone is off](./screenshot.png)

Mute with <kbd>Cmd</kbd> <kbd>Shift</kbd> <kbd>A</kbd> or from the system tray dropdown. This is configurable from a settings file in `~/Library/Application Support/mic-mute/settings.json`.

## Settings

The settings file at `~/Library/Application Support/mic-mute/settings.json` accepts the following keys. Missing keys fall back to defaults, so the file may be partial.

```json
{
  "mic_shortcut": {
    "modifiers": ["shift", "meta"],
    "key": "A"
  },
  "launch_at_login": false,
  "popup_duration_ms": 1000
}
```

| Key | Type | Default | Description |
|---|---|---|---|
| `mic_shortcut.modifiers` | `string[]` | `["shift", "meta"]` | Any of `"shift"`, `"meta"` (Cmd), `"ctrl"`, `"alt"` (Option). |
| `mic_shortcut.key` | `string` | `"A"` | Single key identifier: `"A"`–`"Z"`, `"0"`–`"9"`, `"F1"`–`"F20"`, `"Space"`, etc. |
| `launch_at_login` | `bool` | `false` | Start the app automatically on login. |
| `popup_duration_ms` | `number` | `1000` | How long the on-screen popup pill stays visible after a mute/unmute event, in milliseconds. `0` hides the popup entirely; the menu bar icon still updates. |

The tray menu has **Mute**, **Settings…**, **About**, and **Quit**. The Settings window edits **Launch at Login** and the popup duration; the About window shows the version and an Open GitHub button. Launch at Login and popup duration can also be edited directly in `settings.json`.

## Features

- CoreAudio API mute input devices
  - [x] Mute input devices
    - Note: If native CoreAudio mute is unavailable, Mic Mute falls back to input volume controls, including virtual main volume. Devices exposing neither are skipped.
  - [x] Provide global hotkey muting
  - [x] Poll new devices to mute while microphones should be off
- Visual confirmation of mute status
  - [x] Show microphone mute status in system tray
  - [x] Show microphone mute status in small popup window
  - [x] Popup window shouldn't appear in screenshots or recordings and ignores mouse events
  - [x] Popup follows screens and monitors with cursor
- [x] Add configurable settings (hotkey, startup)
- [x] Open app on system startup

## Limitations

Mic Mute is best-effort, **not** a hardware privacy switch.

- Mutes CoreAudio-controllable devices only.
- Skips devices without mute/volume controls, such as iPhone Continuity Microphone.
- Polling can leave brief mute gaps.
- Drivers can lie; use hardware mute, unplug, or macOS permissions for high assurance.

## Releases

I have not elected to sign the app by joining the Apple Developer Program. The releases have been self-signed by me and can be installed by bypassing the typical app security on macOS. You're also welcome to build and bundle the app yourself with the simple instructions described below.

[View releases](https://github.com/vdsmon/mic-mute/releases)

## Build

Install [mise](https://mise.jdx.dev/) — it manages the Rust toolchain plus dev deps (watchexec, lefthook) and runs project tasks.

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
    -days 3650 -nodes -subj "/CN=mic-mute"
cat sign.key >> sign.crt
rm sign.key
```

Build a release.

```sh
mise run release
```

</details>
