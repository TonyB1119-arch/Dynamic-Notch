# Dynamic Notch

**Replace the Windows shell.** A notch, a dock, a launcher and a tray, drawn in one style and running as one small program.

Dynamic Notch takes over the jobs of the Windows taskbar: it shows what is playing, what your PC is doing and what needs your attention in a notch at the top, it keeps your apps in a dock at the bottom (and can hide the Windows taskbar for you), it finds and starts anything from a launcher, and it lists your tray icons. Everything shares the same flat black glass, thin icons and rows of tiny glowing bars we call **beams**.

Written in Rust. No browser engine, no runtime to install, one `.exe`.

<p align="center"><img src="docs/screenshots/home.png" alt="The home page with media, system stats and weather" width="640"></p>

---

## About Dynamic Notch

Dynamic Notch started as a simple idea: the part of the screen you look at all day should feel calm, quick and a little bit beautiful. The Windows taskbar, the Start menu, the tray and the quick settings each solve a small problem, but they were never drawn together, so they look and behave like four unrelated tools. Dynamic Notch rebuilds those jobs as one program with one visual language. Everything is flat black glass with thin icons, soft corners and rows of tiny glowing bars called beams, and every part follows the same rules for motion and spacing. The result is a desktop that feels designed instead of assembled, and because it is a single small program, it starts quickly, stays out of the way and is easy to quit or update.

The notch is the heart of it. It sits quietly at the top of the screen and does nothing until something happens or you ask it to. When music plays it shows the cover and the title, when you change the volume it slides out a small round peek and then drains away by itself, and when you hover over it or click it, it grows into a full control panel with media, Wi-Fi and Bluetooth, your tray icons, a calendar, a camera mirror, clipboard history, notes, a timer, a downloads watcher and more. Pages slide in and out with springs, staged so the shape moves first and the content follows, which is why opening and closing feels physical rather than abrupt. When you unlock your PC it even greets you, with a ring of beams that fills, a check mark and a handwritten hello, using your name if you have set one.

The dock takes over the bottom of the screen. It holds your pinned apps and every app that is running, magnifies the icons near the pointer, shows a bar under each open app, and shows live previews of an app's windows when you rest the pointer on it. You can click to open, focus or minimize, scroll to step through windows, right-click to pin, unpin or close, and drag icons into the order you like. With "Hide the Windows taskbar" switched on, it becomes a complete taskbar replacement: maximized windows stop above the dock, and the Windows taskbar returns when you switch the option off, quit the app, or start it again after a crash. The dock also gets out of the way by itself when a full-screen app is in front.

The dock is also where the glass look is most visible. It is drawn as liquid glass: Windows blurs what is behind it, you choose how dark the tint is and how strong the blur is (zero gives you perfectly clear glass), and a thin rim along the edge bends the screen behind it the way a thick piece of glass would. To keep that cheap, the bending edge is built from a single picture of the screen taken once, instead of copying the screen over and over, so it costs almost nothing while you work. The trade-off is simple and worth knowing: while the bending edge is on, the dock is hidden from screenshots and recordings, and when you switch it off you get plain clear glass. The look of the rim follows the same glass settings as the notch, so both parts of the shell always match.

The launcher replaces Start menu search and a handful of small utilities. It opens as one glass box in the middle of the screen, either as a wide bar or as a square, and a second box for actions appears only when you need it, for example when you press Ctrl+K to see everything you can do with a result; the card animates smoothly between one and two boxes, and a setting brings back the older two-box layout if you prefer it. Type to find apps, files and quick answers, and press Tab to move between modes: Search, Clipboard, Gemini, Kill, Emoji and Notes. Results use the same icons as your dock, including any custom icons you chose, so an app always looks the same wherever you meet it. Kill mode shows memory and processor load, lists the programs using the most memory, and asks you to press Enter twice before it ends anything, so a stray keystroke can never close the wrong program.

The tray lives in a tab of the notch, which means the corner of the taskbar is no longer needed. Left-click an icon to open its app, and right-click it to get the app's own menu, just like in the Windows taskbar. Around it, the notch quietly handles the small things that used to need separate windows: Windows notifications appear in the notch with a short history, a microphone or camera in use is shown as a peek, a USB drive being connected or removed is announced, caps lock and num lock flash briefly, and finished timers and downloads let you know without stealing focus. Each of these is a module you can switch off, and a module that is off does not run at all, so you only pay for what you actually use.

You can also make it look the way you like. Settings → Appearance offers several looks: Beams, which carries every level and gauge in glowing bars; Calm, which keeps beams only on volume, brightness and battery and uses quieter shapes elsewhere; Minimal, which removes glow and bounce entirely; and Nothing and Bloom, which borrow the dot-matrix style of Nothing OS and the soft, rounded style of the Bloom app. Every feature works in every look, and a look changes the moment you press it. Beyond that, a built-in settings tab covers where the notch sits, which screen it uses, the glass, opacity, sizes, bounce and speed, the accent colour, the home clock, the home widgets, the order of the tabs and your API keys, while a plain, commented config file holds every other option for people who like to edit text.

Finally, it is built to be light and to respect you. It is drawn directly with Direct2D and DirectWrite, with no web view and no browser engine, and it only redraws when something is actually moving, so a resting notch costs next to nothing. Background workers slow down or stop when nobody is looking at them, and idle memory is trimmed after it has been quiet for a while. The camera and microphone stay off until you ask for them, nothing goes online unless you use a feature that needs the internet and has a key, and your clipboard history, notes and shelf stay on your own PC. Updates are never checked in the background: you press the button in Settings, the app asks GitHub once, and if there is something newer it installs quietly and restarts. If you would rather build it yourself, it is a standard Rust project that compiles to a single executable.

---

## One shell, four parts

### The notch

Hangs at the top of the screen and grows into a control panel when you need it: media, Wi-Fi and Bluetooth, your tray icons, calendar, camera mirror, clipboard, notes and more. Volume, battery and other changes show up as a small round peek and go away by themselves.

<p align="center"><a href="docs/media/notch.mp4"><img src="docs/media/notch.gif" alt="The notch: Bluetooth and Wi-Fi, tray, calendar, mirror, media, battery and volume (3x speed)" width="640"></a></p>
<p align="center"><sub>3x speed. <a href="docs/media/notch.mp4">Watch the full video</a></sub></p>

### The launcher

Two glass boxes in the middle of the screen. Type to find apps, files and quick answers; press <kbd>Tab</kbd> to switch what it searches: **Search**, **Clipboard**, **Gemini**, **Kill**, **Emoji** and **Notes**. Every result has actions (open, run as administrator, show in folder, quit, restart, force quit, uninstall). Kill mode shows memory and processor load, lists the programs using the most memory, and asks you to press Enter twice before it ends anything.

<p align="center"><a href="docs/media/launcher.mp4"><img src="docs/media/launcher.gif" alt="The launcher: actions, AI, web search, clipboard, kill and notes (3x speed)" width="560"></a></p>
<p align="center"><sub>3x speed. <a href="docs/media/launcher.mp4">Watch the full video</a></sub></p>

### The dock

A dock for the bottom of the screen with your pinned and running apps, magnified icons, live window previews and a right-click menu. Turn on "Hide the Windows taskbar" and it becomes a full taskbar replacement; the Windows taskbar comes back when you switch it off or quit.

<p align="center"><a href="docs/media/dock.mp4"><img src="docs/media/dock.gif" alt="The dock with pinned apps and its right-click menu" width="640"></a></p>
<p align="center"><sub><a href="docs/media/dock.mp4">Watch the video</a></sub></p>

### The tray

Your tray icons live in a tab of the notch, so the corner of the taskbar is no longer needed. Click an icon to open its app.

### What it replaces

| In Windows | In Dynamic Notch |
|---|---|
| Taskbar | The dock, optionally hiding the Windows taskbar |
| Start menu search | The launcher |
| Notification area | The Tray tab |
| Quick settings (Wi-Fi, Bluetooth, volume, battery) | The notch |
| Clipboard history, Task Manager, emoji panel | Launcher modes: Clipboard, Kill, Emoji |
| Sticky notes | The Notes mode and the Notes tab |

File Explorer still manages your files, and Windows still runs underneath. Dynamic Notch replaces the parts you look at and reach for all day.

---

## At a glance

| | |
|---|---|
| **Look** | Black glass, soft corners, hairline dividers, blue beams that shine white at their leading edge |
| **Motion** | Spring-driven open and close, staged so the shape moves first and the content follows |
| **Size** | One small executable, no installer, no background services |
| **Settings** | A built-in settings tab, plus a plain-text config file for everything else |
| **Platform** | Windows 10 and 11 |

---

## What is inside

### The home page

- A **rolling-digit clock** with seconds (or a flip clock, your choice), with AM/PM beside it.
- A **calendar strip** under the time. Scroll the mouse wheel over it, or click a day, to look through other dates. The chosen day is blue and a lit beam glides along a rail beneath it. Click the month to jump back to today.
- **Up to four widgets** you pick yourself: system stats, weather, battery, devices and more.
- When music plays, the player takes the left side and the clock tucks into a slim strip underneath.

### The tabs

| Tab | What it does |
|---|---|
| **Media** | Now playing for any app that reports to Windows. A spinning record with the cover as its label, a beam scrubber, a live visualizer |
| **System stats** | Three beam rings for CPU, memory and graphics |
| **Timer** | Countdown and stopwatch with quick presets; the notch shows the time left while it runs |
| **Clipboard** | History of what you copied, newest first, with a beam showing how fresh each entry is |
| **Notes** | Small snippets you can copy back with one click |
| **Shelf** | Drop files on the notch to park them, drag them out when you need them |
| **Downloads** | Watches your Downloads folder and shows what just arrived |
| **Weather** | Current conditions and a short forecast, no account or key needed |
| **Wi-Fi and Bluetooth** | Networks and paired devices in one place, with signal beams and device batteries |
| **Notifications** | Windows notifications appear in the notch, with a short history |
| **Dev servers** | Lists what is listening on localhost, with page titles, so you can open or copy a link |
| **Dictation** | Hold a hotkey, speak, release; the text is pasted into the app you were typing in |
| **Ask AI / Gemini** | Chat panels for quick questions, with your own API key |
| **Mirror** | Press the round button to see yourself in the camera. It only runs while you ask for it |
| **Settings** | Everything below |

A few of the tabs:

<p align="center"><img src="docs/screenshots/stats.png" alt="System stats" width="560"></p>
<p align="center"><img src="docs/screenshots/weather.png" alt="Weather" width="560"></p>
<p align="center"><img src="docs/screenshots/clipboard.png" alt="Clipboard" width="560"></p>
<p align="center"><img src="docs/screenshots/ask-ai.png" alt="Ask AI" width="560"></p>
<p align="center"><img src="docs/screenshots/mirror.png" alt="Mirror" width="560"></p>

### Peeks and indicators

<p align="center"><img src="docs/screenshots/closed-notch.png" alt="The closed notch with album art" width="420"></p>

Small, short-lived messages that slide out of the closed notch and drain away on their own:

- volume and brightness, drawn as a long beam bar (rest the pointer on the speaker to switch output device)
- caps lock and num lock
- a USB drive connected or removed
- a new track, a finished timer, a finished download
- a microphone or camera in use
- the **unlock greeting**: when you unlock your PC, a ring of beams fills, a check mark confirms, then "hello" is written in handwriting, with your name if you set one

<p align="center">
  <img src="docs/screenshots/peek-notification.png" alt="A Windows notification peek" width="310">
  <img src="docs/screenshots/peek-clipboard.png" alt="A clipboard peek" width="310">
</p>
<p align="center">
  <img src="docs/screenshots/volume.png" alt="Volume" width="310">
  <img src="docs/screenshots/brightness.png" alt="Brightness" width="310">
</p>

---

## Looks

Settings → Appearance → **Look** has four choices:

- **Beams** (default): rows of small glowing bars carry every level, progress and signal: volume and brightness, the media scrubber, system stats, signal strength, battery gauges, downloads, the timer and more.
- **Calm**: beams stay only on the volume and brightness bars and the battery gauges. Everything else is one of three quieter looks: a **glow bar** whose lit end turns white and swells gradually (progress, sliders, ranges, shimmers), **pill segments** (small meters and graphs) and a **dot ring** (every circular gauge).
- **Minimal**: no glow and no swell. Gauges are thin plain lines, rings are hairline arcs, "working on it" is one short line gliding along its rail, and the scrubber shows a small dot only when you hover it. Motion has no bounce at all, so everything arrives and settles quietly.

- **Nothing**: inspired by Nothing OS. Everything is white on black with a single red accent, drawn as dot-matrix "glyph" lights. Gauges are rows of LED dots with a red leading dot, "working on it" is a little comet of dots stepping along, rings are circles of dots, graphs are columns of dots, the tab marker is one red dot, and the home clock is a 5x7 dot-matrix display with a red colon. Motion is crisp, mechanical and bounce-free.
- **Bloom**: soft and rounded, in the spirit of the Bloom AI app. Gauges are thick capsules on a faint track, rings are thick with round ends, graphs are rounded bars, the scrubber is a capsule with a round knob that appears on hover, and lists and cards sit in bordered, translucent groups. The accent is a clear iOS blue, and motion is a smooth ease-out with hardly any overshoot. Everything else (Gemini, clipboard, shelf, ports and the rest) picks the look up automatically.

Every feature is available in all four. The look changes as soon as you press it.

How beams behave:

- Lit beams are blue; unlit ones are a quiet grey.
- The beams at the leading edge shine white, and **the whiter a beam is, the larger it is**.
- Values glide to their new level instead of jumping.
- Beams are only redrawn while something is moving, so a resting notch costs next to nothing.
- Prefer white beams? There is a switch for that in Settings.

---

## Install and update

**Installer (easiest).** Download `DynamicNotch-Setup.exe` from the Releases page and run it. It installs for your user only, so there is no administrator prompt, and it adds a Start menu shortcut and an uninstaller. It installs no service and no scheduled task: whether the notch starts with Windows is a setting inside the app.

**Portable.** The release zip holds a single `dynamic-notch.exe`. Unzip it anywhere and run it.

**The dock.** Switch it on under Settings → Dock. It sits at the bottom of the screen with your pinned apps and every app that is running: icons grow near the pointer, a bar under the icon shows what is open (a long one for the focused window), resting the pointer on an app shows live previews of its windows (click one to switch, the small cross closes it), click an icon to open, focus or minimize, scroll over it to step through windows, right-click to pin, unpin or close, and drag icons to reorder. "Hide the Windows taskbar" makes it a full taskbar replacement: maximized windows stop above the dock, and the taskbar comes back when you switch it off, quit, or start the app again after a crash. The dock hides itself while a full-screen app is in front. `[dock]` in the config.

**Updating.** Open Settings → General → Updates and press **Check for updates**. The app asks GitHub for the newest release once, then stops. If there is a newer one, press **Install**: it downloads the installer, runs it quietly, and the notch restarts on the new version. Nothing is ever checked in the background or at start-up. The repository it looks at is `[update] repo` in the config.

**Build it yourself.** You need Windows and the Rust toolchain:

```
cargo build --release
```

The program ends up in `target\release\dynamic-notch.exe`. To build for Windows from another system:

```
cargo build --release --target x86_64-pc-windows-gnu
```

To make the installer yourself, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and compile `installer/DynamicNotch.iss`. Pushing a release tag such as `v3` builds the zips and the installer on GitHub automatically.

---

## Using it

**Mouse**

- Hover over the notch (or click it) to open it. Hover opening can be turned off in the config.
- Click a header icon to change tab. Scroll the wheel over the header to flip through tabs.
- Right-click the notch or the tray icon for the menu.

**Keyboard** (all of these can be changed)

| Keys | Action |
|---|---|
| `Ctrl+Alt+N` | Open or close the notch |
| `Ctrl+Alt+Space` | Ask AI |
| `Ctrl+Alt+T` | Timer |
| `Ctrl+Alt+V` | Clipboard |
| `Ctrl+Alt+D` | Dictation (tap to start and stop, or hold to talk) |

**Command line**

A second launch talks to the copy that is already running:

```
dynamic-notch toggle | expand | collapse | ai | reload | quit
```

---

## Settings

Open the gear in the header. The tabs cover:

- **General**: where the notch sits, which screen, what it does when idle, the unlock greeting, the name it says hello to, the hello font, a few handy buttons (edit the config file, open the data folder), and the **Updates** button.
- **Look**: glass, opacity, sizes, bounce, speed, accent colour, beam style, the home clock style, and a switch for lighter animations.
- **Home**: choose and order the home widgets.
- **Modules**: switch whole features on or off. A feature that is off does not run at all.
- **Weather**: your city and units.
- **Tabs**: reorder the header tabs.
- **Gemini**: paste your key.

Changes apply straight away.

---

## Config and keys

Everything lives in `%APPDATA%\DynamicNotch\`:

| File | Purpose |
|---|---|
| `config.toml` | Every option, commented. Edit it any time |
| `.env` | API keys, kept apart from the config so you can share your settings safely |

Keys the app understands:

| Key | Used by |
|---|---|
| `GEMINI_API_KEY` | Gemini chat |
| `OPENROUTER_API_KEY` | Ask AI |
| `GROQ_API_KEY` | Dictation |

You can paste the Gemini key straight into the app; the others go in `.env`.

---

## Privacy

- The camera and microphone are off until you ask. The Mirror closes itself when you leave its tab or close the notch.
- Nothing goes online unless you use a feature that needs the internet (weather, AI, dictation) and have given it a key where one is needed.
- Clipboard history, notes and the shelf stay on your PC.
- API keys are stored in `.env`, never in `config.toml`.

---

## Built to stay light

- Drawn directly with Direct2D and DirectWrite. There is no web view.
- The notch only redraws when something changes. Ambient effects (shimmers, sweeps) run at your screen's refresh rate up to 60 fps, and nothing is drawn while everything is still.
- Opening, closing and page slides draw the finished content once as a picture and just move it, which keeps those moments cheap. This can be turned off in Settings.
- Text layouts, images and shapes are cached with size limits, and idle memory is trimmed after the notch has been quiet for a while.
- Background workers (stats, dev servers, notifications) slow down or stop when nobody is looking at them.

---

## For developers

```
src/
  main.rs, window.rs, host.rs   the window and the compositor surface
  app.rs                        the notch itself: layout, springs, drawing
  anim.rs, pacer.rs             springs and frame pacing
  gfx/                          Direct2D / DirectWrite painter and icons
  ui.rs                         buttons, beams, clocks and other widgets
  config.rs                     settings and the default config template
  modules/                      one file per feature
  sys/                          Windows helpers (audio, clipboard, HTTP, speech)
```

**Adding a feature** means adding a file in `src/modules/` that implements the `Module` trait: an id, a title, an icon, and what to draw. A module can have a page, a home widget, a peek, or an indicator on the closed notch, and it receives system events and messages from background threads through a small message bus. Register it in `modules/mod.rs`, add its switch to the config, and it shows up in the header.

Run the tests with `cargo test`.

---

## Credits and licence

Built on the open-source DynamicNotch project by eigenes (MIT), with a new look, many new modules and a lot of polish on top. See `LICENSE`.
