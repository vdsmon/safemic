# SafeMic

macOS tray app that mutes every input device system-wide via a global hotkey, with a transient on-screen confirmation.

## Language

**Mute shortcut**:
The single global hotkey that toggles microphone mute.
_Avoid_: hotkey, keybinding

**Shortcut chip**:
The control in Settings that displays the Mute shortcut and records a new one when clicked.
_Avoid_: well, recorder button

**Popup**:
The transient icon-only bezel (volume-OSD style) that confirms the microphone state after a toggle, then auto-hides.
_Avoid_: HUD, toast, notification

**Popup duration**:
How long the Popup stays visible, in seconds with 0.1s resolution. 0 means the Popup never shows.

**Auto-apply**:
Every settings change persists and takes effect immediately; there is no Save button. Success is silent; only failures produce feedback.

**Footer**:
The single caption line beneath the settings card that carries transient text: recording instructions, shortcut conflicts, save errors. Empty in the idle state.

**Semantic red**:
Red appears only where it means something — the muted state, warnings, save errors, and the app icon. Never as decoration or accent; interactive controls use the user's system accent color.
_Avoid_: brand red, accent red

**Enforce mute**:
The 200ms poll that re-asserts mute on newly plugged or reappearing input devices while muted.
