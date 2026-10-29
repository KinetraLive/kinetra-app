# Kinetra on macOS

Kinetra runs on macOS 11 (Big Sur) or later, on Apple silicon and Intel Macs. Windows needs
Windows 10 or later.

Most of Kinetra works the same on both systems: templates, screens, Control, Studio mode,
sources (data tables, schedules, CSV and JSON links, API sources, live data), flows and recipes,
live polls, AI templates, display outputs, MIDI and OSC control, sign-in and licensing, updates.
**Browser sources for OBS and vMix work on macOS** exactly as on Windows.

## Not available on macOS yet

| Feature | On macOS today |
|---|---|
| **NDI® output** (sending a screen as an NDI® source) | Not available yet. Use browser sources or a display output, or receive NDI® from a Windows computer running Kinetra. |
| **Choosing an output's audio device** | A display output's sound plays on the Mac's current sound output (System Settings › Sound). |
| **Recovering a crashed output window** | Not available yet: if an output page stops, turn the output off and on. |

Projects are the same on both systems: a project made on Windows opens on a Mac and the other
way round. Display outputs and MIDI device settings (inputs, LED feedback) are set up per
computer.

## Different on macOS

- **Display outputs** cover their whole display, including the menu bar and the Dock there, and
  stay put when you switch Spaces or another app goes full screen. They don't take focus from
  the Kinetra window you work in. While an output shows, the displays don't go to sleep.
- **Fill and key on two displays:** with "Displays have separate Spaces" on (System Settings ›
  Desktop & Dock, on by default), each display gets its own window, so fill and key aren't
  frame locked. Turn the setting off (and log out and in) to get one frame-locked window over
  both displays, as on Windows.
- **Retina and scaled displays:** an output display set to a scaled resolution ("looks like" a
  size the panel doesn't have) shows "Scaled display mode: not pixel exact" in Preferences ›
  Outputs. For pixel-exact graphics, set that display to its default resolution in System
  Settings › Displays. An external 1920×1080 display or a projector at its native resolution is
  pixel exact.
- **The mouse pointer** may still show over an output display while another app is in front.
  Move it off the output display before the show.
- Secret header values (API keys in API sources) are kept in the **macOS Keychain** (Windows:
  Credential Manager). Like on Windows, they stay on the computer they were entered on.
- Some video files play differently in the Mac's web engine than on Windows (for example WebM
  with transparency). The media library shows a warning for each file that won't play on a
  platform, with a command to convert it.
- **MIDI devices** are found the same way as on Windows, including controllers plugged in while
  Kinetra runs, Bluetooth MIDI and network MIDI sessions set up in Audio MIDI Setup. A mapping
  made on Windows works on a Mac and the other way round, even though the two systems name some
  controllers differently (for example "APC mini mk2" on Windows, "APC mini mk2 Control" on a
  Mac). If a controller has two ports with similar names, learn the mapping again.
- The menu has the usual Mac app menu: About Kinetra, Preferences… (⌘,) and Quit.
