# Sending screens over NDI®

Kinetra can send any screen as an **NDI® source** on your network, with transparency. vMix, OBS
(with the DistroAV plugin), NDI Studio Monitor and other NDI® tools find it like any other source
and put your graphics over their cameras, without a browser source. NDI® output is part of every
plan, Free included.

NDI® output works on **Windows 10 and later**. It isn't available on macOS yet
([Kinetra on macOS](MACOS.md)).

## What you need

- The free **NDI® Runtime** on the computer running Kinetra. Kinetra doesn't include it. Get it
  from [ndi.link/NDIRedistV6](https://ndi.link/NDIRedistV6) (Kinetra's **Download** link in
  Preferences › Outputs goes there), or install **NDI Tools**, which includes it. Kinetra finds it
  when you come back to Preferences › Outputs, without a restart. If it still says the Runtime
  isn't installed (for example because you installed it to another folder), restart Kinetra.
- The computer that receives the source on the same network (or the same computer).

## Set up an NDI® output

1. Open a project, then **Preferences › Outputs › Add NDI® output**.
2. Pick the **Screen** to send and the **Frame rate** (see the next section).
3. Turn the output on. Its status reads **Ready · no receiver**.
4. In your production software, add the NDI® source. It's listed as
   `YOUR-PC (Kinetra Main)`: your computer's name, then the output's name. Click the name in
   Kinetra to change it.
   - **vMix:** Add Input › NDI / Desktop Capture, then pick the source.
   - **OBS:** with [DistroAV](https://github.com/DistroAV/DistroAV) installed, add an **NDI™ Source**.
   - **NDI Studio Monitor:** pick the source from its menu.

Once something receives it, the status changes to **Sending**. vMix also sends tally back (as do other receivers that support NDI® tally): the
status shows **On preview** (green) or **On program** (red) when the source is in vMix's Preview
or Program.

You can also turn an output on from the command palette (Ctrl+K): **Send Main over NDI®** and
**Stop sending Main over NDI®** (with your screen's name). If the screen has no NDI® output yet,
Kinetra makes one.

You can have up to 4 NDI® outputs, each sending one screen. They're set up per computer, like
display outputs: they aren't saved in the project.

## Match your production's frame rate

**Set the output's frame rate to the frame rate of your vMix or OBS project.** When they differ,
the receiving software has to convert the rate: frames get dropped or doubled and motion
(tickers, wipes, clocks) stutters. In our tests a 50 fps output into a vMix project at 50 fps
dropped no frames over 30 minutes; into a project at another rate it dropped hundreds.

Kinetra suggests 50 fps in Europe and 59.94 fps elsewhere, from your Windows region. The choices
are 25, 29.97, 30, 50, 59.94 and 60 fps.

## Settings

| Setting | What it does |
|---|---|
| **Screen** | The screen this output sends. The source has the screen's size, e.g. 1920 × 1080. |
| **Frame rate** | See above. |
| **Transparency** | On: graphics arrive with transparency, ready to overlay. Off: they arrive on the **Background** color you pick, and use a little less network bandwidth. |

In **Advanced** mode (Preferences › Advanced) you also get:

| Setting | What it does |
|---|---|
| **Alpha** | **Straight** (the default) or **Premultiplied**: how transparency is encoded. vMix and OBS show graphics correctly with Straight. If edges or shadows look too dark or too light in another receiver, try Premultiplied. NDI Studio Monitor expects Premultiplied, so with Straight its preview looks slightly off; what vMix and OBS show is what counts. |
| **Bus** | **Program** (the default) or **Preview**. With Studio mode on in Control, Preview sends what's cued there, so a receiver can show your next graphics before you take them. |
| **NDI® groups** | Receivers see the source only when they look in one of these groups. Empty means the default group, Public. |

Changing a setting restarts that output for a moment; receivers reconnect by themselves.

## While it's on air

An NDI® output counts as **on air while something receives it**:

- **Exit** asks first ("1 output is showing. Exit anyway?").
- **Updates** wait: Kinetra doesn't install one while an output is showing.
- **Cloud backup** pauses (or slows down, if you chose that) while a receiver is connected.

With no receiver connected the output does almost no work, so you can leave it on.

If the output can't keep up (a busy computer, a slow network), the status shows **Dropping
frames**. If the page that draws the output crashes, Kinetra restarts it by itself; if it keeps
crashing, the status says **The NDI output page keeps crashing**: turn the output off and on.

## Older Windows: less even motion

Kinetra captures the screen exactly at the output's frame rate on **Windows 11 version 24H2 and
later**. On **Windows 10** and **Windows 11 before 24H2**, Windows captures at about 60 frames a
second whatever Kinetra asks for, so the frames sent don't line up exactly with the output rate:

| Output | Windows 11 24H2 and later | Windows 10, Windows 11 before 24H2 |
|---|---|---|
| 50 fps | smooth | about 1 frame in 7 jumps ahead slightly |
| 59.94 fps | smooth | about 1 frame in 30 repeats or jumps ahead |
| 60 fps | smooth | about 1 frame in 16 repeats |

Graphics that don't move (a lower third that's in, a bug, a score) look exactly the same; only
motion such as a ticker or a wipe is a little less even. To see your version: Settings › System ›
About › Windows specifications.

## Known limitations

- **Video layers can stutter slightly.** Videos play inside the screen's page, and a few frames in
  a hundred are repeated or skipped over NDI®. Graphics are not affected. A browser source or a
  display output shows video as before.
- **No audio.** NDI® outputs carry video only. Use a browser source or a display output for media
  audio.
- **The source has the screen's size.** To send another size, change the screen's size.

## Not tested yet

These should work, but we haven't tested them yet. If you use Kinetra this way, we'd like to hear
how it went:

- **The network dropping out while sending** (a cable pulled, Wi-Fi lost). Receivers should
  reconnect by themselves when the network is back.
- **A computer whose display runs at 60 Hz.** Kinetra was tested on displays at 144 Hz and more;
  at 60 Hz, motion may be less even at 50 fps, like on older Windows above.
- **A laptop whose built-in graphics drive the display** while a separate graphics card is also
  installed. Kinetra uses the graphics chip that drives the main display.

[About NDI®](https://ndi.video)

NDI® is a registered trademark of Vizrt NDI AB.
