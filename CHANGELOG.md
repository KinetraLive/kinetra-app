# Changelog

Release notes for Kinetra, newest first, shown in Help › Check for updates. See docs/RELEASING.md.

- "Unreleased" is the next canary's notes: only what's new since the canary that is out. Write it
  before each canary; after it ships, move its paragraphs into "Next stable release" and leave
  "Unreleased" empty. (A nightly canary without new notes isn't built.)
- "Next stable release" collects everything since the last stable release. A stable release
  vX.Y.Z renames it to "X.Y.Z" (rewritten for it) and starts a new empty one.

Written for Kinetra's users (what they gain, not internal changes) and shown as plain text in the
update panel: one paragraph per line, no Markdown.

## Unreleased

New template fits on any screen: pick a look and its name is ready to type, with Create always in view, so Enter makes it. Double-click a look to create it straight away. Long rules keep Save in view too, and scrollbars are easier to spot in the dark theme.

Selecting text in a dialog and letting go outside it no longer closes the dialog. Dialogs also cover what's behind them from the moment they open: the red on-air line of a screen no longer shows through New cue or Add automation the first time you open them.

API sources: after a test, each column's field suggests the columns the response has (CSV header names or column numbers, JSON paths from the rows), and you can still type your own. A column the latest test doesn't have gets a clear warning.

Row numbers in flows now count from 1, like everywhere else; a flow that wires a fixed row number will now go to exactly that row (one earlier than before). Step source shows the row it goes to ("Go to row 75"), and an armed flow with changes you haven't deployed now says clearly that it still runs the old version, with Deploy now right there.

## Next stable release

Typing a license key by hand works again: type it with or without "KNTR-", in any case, with spaces or without dashes. Kinetra tidies it when you paste it or leave the field.

AI templates (Pro and Team) come with neatly formatted HTML and CSS, and a new Format HTML and CSS command (Alt+Shift+F) tidies your own code. The editor previews generated templates with realistic example text and numbers, while new layers start empty, so examples never go on air by accident.

WebSocket support, in Advanced mode: a Live data (WebSocket) source connects to a WebSocket feed on this computer or your network and maps its JSON to your graphics' fields. Graphics update live as the data changes, and the "API value changed" flow trigger works with these sources too.

Remote control (Pro and Team): run your show from a phone, tablet or another computer. Turn it on for a project, invite collaborators by their Kinetra account and choose what each person may do, from full operator to editing text or just watching.

Collaborators see a live picture of what's on air (and Preview in Studio mode), take and clear graphics, edit text, run cues, step through data sources, switch looks, start timers and take Preview. Build a button panel in Kinetra for one-tap actions on their screen. Graphics on air are never interrupted: the live picture is the first thing to pause when your computer is busy.

Send screens over NDI® (Windows, every plan): add an NDI® output in Preferences › Outputs and the screen shows up in vMix, OBS and other NDI® tools on your network, with transparency and tally. Setup, frame rates and known limits: docs.kinetra.live/ndi

NDI® is a registered trademark of Vizrt NDI AB.

Cloud backup (Pro and Team): your projects back up to the cloud while you work, and each backup is kept as a version you can go back to. Restore an older version into a new folder or over the project, or get a project onto another computer with In the cloud on the start screen. Backups never get in the way of a show: they pause while anything is on air (or go slowly, if you prefer), and every restored file is checked before it's used.

Control Kinetra from Bitfocus Companion and Stream Deck: the Kinetra module is now in Companion (search for "Kinetra" when you add a connection). Turn on Preferences › Network & API › Control API and copy the token into it. Buttons take and clear graphics, change their text, run cues, step sources and take Preview, and light up red on air and green in Preview. Scripts can use the same API, and a flow can start from an API call, for example to put a score on air from one button.

Signed for Windows: the installer and the app now show Rørvik Medieteknikk as the publisher.

The cue strip in Control marks the next cue: the one after the cue you ran last shows Next.

Live polls: run audience polls from the new Polls page and show them on screen. Viewers vote on their phones with a QR code or a short link, and the results update live on air. The Poll graphic comes in all nine styles, with looks for Live, Starting soon, Voting closed and Results, and the winner marked when voting ends.

Control fits smaller windows: when space runs out, the less-used toolbar options move into a ⋯ menu, so PANIC always stays in reach.

Clearer screen tiles: selected, Preview and Program now show as a simple line around each screen (grey, dashed green, solid red), always next to the on-air dot.

Clear screen: the button in Control's toolbar is now called Clear screen, because it clears the selected screen. Show › Clear all still clears every screen.

Better previews: the template gallery shows realistic sample text, and empty fields in the template editor show what belongs there, like [Name].

Your license, simpler: add a license key in Preferences › License, not only on the welcome screen. Your plan shows by its name, like Kinetra Team, and a backup computer is only offered when your plan includes one.

Your license now stays put: Kinetra keeps it with its own settings, so it works even when Windows Credential Manager won't store it, and your existing license moves over by itself. If Kinetra can't save it, Preferences › License says why. Check now shows the result, and adding a license key tells you whether this computer switched to it.

Help › Documentation now opens docs.kinetra.live.

Auto-out can be set up to 600 seconds.

Better on Mac: schedules and time-of-day automation follow your Mac's time zone, Pro can be activated on a Mac, Kinetra shows your Mac's name in your account, and secret API keys say they're kept in the macOS Keychain. Templates, screens and sources you edit in another program now update in Kinetra on a Mac too. Display outputs and MIDI aren't available on macOS yet; their settings say so and link to what works on Mac.

Kinetra now needs Windows 10 or later, or macOS 11 or later.
