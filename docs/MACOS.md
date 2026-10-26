# Kinetra on macOS

Kinetra runs on macOS 11 (Big Sur) or later, on Apple silicon and Intel Macs. Windows needs
Windows 10 or later.

Most of Kinetra works the same on both systems: templates, screens, Control, Studio mode,
sources (data tables, schedules, CSV and JSON links, API sources), flows and recipes, live
polls, AI templates, OSC, sign-in and licensing, updates. **Browser sources for OBS and vMix work
on macOS** exactly as on Windows: that is the way to get Kinetra graphics into your production
on a Mac today.

## Not available on macOS yet

| Feature | On macOS today |
|---|---|
| **Display outputs** (full-screen fill / key on a connected display, test patterns, Identify) | Not available. Preferences › Outputs finds no displays. Use browser sources instead. |
| **MIDI control** (MIDI mappings, Learn, LED feedback, the MIDI in flow trigger) | Not available. No MIDI devices are found and mappings don't run. OSC works and can drive the same actions. |
| **Output audio** (choosing the audio device of a display output) | Not available (display outputs aren't). Browser sources play their audio in OBS / vMix as usual. |
| **Recovering a crashed output window** | Not available (display outputs aren't). |
| **Hot-plug** (displays and MIDI devices picked up while Kinetra runs) | Not available (display outputs and MIDI aren't). |
| **NDI® output** (sending a screen as an NDI® source) | Not available yet. Use browser sources, or receive NDI® from a Windows computer running Kinetra. |

Projects are the same on both systems: a project made on Windows opens on a Mac and the other
way round. A project's MIDI mappings are kept on a Mac and run again when the project is opened
on Windows. Display outputs are set up per computer.

## Different on macOS

- Secret header values (API keys in API sources) are kept in the **macOS Keychain** (Windows:
  Credential Manager). Like on Windows, they stay on the computer they were entered on.
- Some video files play differently in the Mac's web engine than on Windows (for example WebM
  with transparency). The media library shows a warning for each file that won't play on a
  platform, with a command to convert it.
- The menu has the usual Mac app menu: About Kinetra, Preferences… (⌘,) and Quit.
