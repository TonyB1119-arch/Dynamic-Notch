# Changelog

All notable changes to Dynamic Notch are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- **Mirror has its own tab again**: the round camera button no longer sits on the home page (where it overlapped the calendar strip). Open the Mirror tab (camera icon in the header), press the button and the camera view grows to fill the tab; it switches off when you leave the tab or close the notch. Reorder or hide it under Settings → Tabs / Modules.
- **Calendar strip on the home clock**: under the time the date is replaced by a week strip: the month in large type, then the days. Scroll the mouse wheel over it (or click a day) to look through other dates; the chosen day is blue with a lit beam gliding along a rail of beams, the other days are white (days before today dimmer). Click the month, or "today", to jump back. The "Nothing playing" line is gone, and AM / PM now sits next to the clock.
- **Less work per frame**: the drawing code no longer copies the whole appearance settings (with their text fields) on every frame; it reads just the few values it needs.
- **One beam look everywhere**: every beam now sits on whole device pixels with a whole-pixel width, so no beam looks lighter or thinner than its neighbours. The whiter a beam shines, the larger it is: the leading edge of every gauge, the sweeps, the rings and the newest bars of the mini graphs grow a little as they turn white. Gauges and rings ease in and out through two gentle glides, which makes the motion smoother (media scrubber, clipboard, notifications, volume and brightness, Gemini, weather, shelf, downloads and the rest all use these shared beams).
- **Volume and brightness bar**: smoother (the level eases in and out through two gentle steps) and blue with clearer white highlights: the leading beams shine white and grow slightly, with a faint glint on every seventh beam. Muted stays quiet.
- **Sharper, wider Mirror**: the camera preview is now kept at up to 640 px wide (was 320) and shrunk by averaging instead of skipping pixels, so it no longer looks blocky. It also shows more of the picture (slightly zoomed out) instead of cropping the edges; change `ZOOM` in `modules/mirror.rs` (1.0 = fill, 0.0 = whole picture).
- **Sturdier Downloads watcher**: the folder-change parser now stops at the end of the data Windows returned instead of trusting each record, so a damaged record can no longer cause a crash.
- **Cached open/close hands over sooner**, so clicks work as soon as they did before the cached frames were added.
- **Notification check every 2 seconds** instead of 1.5, a little less background work.
- **Hello font switch**: Settings → General has a new "Hello font" row to switch the unlock greeting between Yellowtail and Script Regular (the font must be installed; otherwise the built-in pen drawing is used).
- **Cached frames for big motions**: while the notch opens or closes, and while a page slides, the finished content is drawn once into a GPU picture and each frame just moves and fades that picture, instead of laying out and drawing every icon, text and beam again. The live view takes over the moment the motion settles. Turn it off under Settings → Look → "Light animations (cached frames)".
- **Staged open and close, one spring for the whole notch**: closing now fades the content out first and only then shrinks the body around the empty space; opening lets the body lead and the content follow. The body's width, height, corner radius and the top ears now share one spring character, and the page slide uses the same timing, so it all moves like one object.
- **Gemini beams line up with your text**: the rule of beams under the message field now ends exactly under the last character you typed (and glides there as you type), instead of filling by character count.
- **A bit less work per frame**: the header no longer builds a text copy for every tab on every frame, and the battery label is worked out when the battery changes instead of on every frame.
- **Lighter at rest and in motion**: ambient animations (sweeping beams, the turning record) now draw about 30 frames a second while anything that is actually moving or being touched still runs at full speed; after ten idle seconds the app also hands its freed memory back to Windows. The flip clock lost its centre line.
- **A calmer look everywhere**: the header tabs are bare icons now, and the open one is bright with a short lit beam under it (no white pill). Volume and brightness fit on one quiet row: icon, beams, number. Text popups lose their coloured status badge and are a little shorter; they keep the draining beam rail. Section labels, the tinted status circles in the header and the box around the Ask AI field are gone, leaving thin dividers and beams to do the separating.
- **Cheaper tab switching**: a switch now draws one page at a time (the old page slides a little and fades out, then the new one slides in and fades up) instead of both at once, with whole-pixel movement and a slightly shorter spring. In the header the tab icons keep one size while the pill grows, so their shapes are no longer rebuilt on every frame of the animation.
- **Less memory**: the text, shape and image caches now keep things for about half as long and are smaller, and after ten idle seconds everything that can be rebuilt on demand (text layouts and cached shapes included) is released.
- **Lighter on CPU and memory**: finished text layouts are now kept and reused (the same text in the same box used to be laid out again on every frame, which was the biggest single cost while anything was on screen); the Gemini chat no longer copies the whole conversation on every frame; the clock works out its date and time text once a minute instead of every frame; the weather forecast is shared instead of copied on every frame. The layout cache is bounded and trims itself, so memory stays flat.
- **New Gemini tab**: no boxes any more. The conversation is plain text on black: what you asked is small and dim with a thin beam beside it, Gemini's answer is larger and bright. You type on a single line with the rule of beams under it (it fills as you type, sweeps while Gemini answers and otherwise shows how much of the chat Gemini sees), with small quiet icons for send, copy, new chat and your key at the end. The key page uses the same single line.
- **Tab bar animation**: switching tabs now animates the header. The open tab's white pill grows out of its icon while the name fades in, the tab that was open shrinks back to a round icon, and the others slide along.
- **New weather tab**: the weather now across the top (icon, temperature, condition, place, feels like, and humidity and wind as beam gauges) and the five-day forecast below it as flat columns: day, icon, a range of beams from low to high, the low and high, and the chance of rain.
- **Mirror**: the round button is bigger, and the camera view now fills the whole notch below the header, edge to edge.
- **Next track**: only the first track change of a listening session shows a peek. After that, the open peek swaps to the new track with a short animation (the old title drifts up and fades while the new one rises in), and nothing new pops up. The player card on the home page animates the same way, with the record dipping while its label changes.
- **Battery**: next to the battery in the header, the time left is shown in blue (e.g. "82%  2h 15m") and a drawn blue infinity sign while plugged in. When Windows can't estimate the time, it is worked out from how fast the charge has dropped since unplugging (after a couple of percent), and refreshed every 45 seconds.
- **Rounded icons**: every corner in the icon set is now rounded off (house, arrow heads, bolt, send, package, Bluetooth, the sparkle and the battery bolt).
- **New weather icons**: the weather conditions are white line art in the same style as the rest of the icons (sun, moon, clouds, fog, drizzle, rain, snow, storm). Only water is blue: the rain, the drizzle and the snow.
- **Blue accent on rings and graphs**: in the white beam style, the tick rings (CPU, memory, GPU and every other gauge) and the Stats history graphs now carry the blue accent at their leading edge too.
- **Quieter Gemini tab**: the header, the context beams, the footer and the dividers between messages are gone. What is left is the conversation and a single row: the message field with small icons beside it for your key, copy, a new chat and send. A single rule of beams sits above the field: it sweeps while Gemini answers, fills as you type, and otherwise shows how much of the chat Gemini can see.
- **Mirror lives on the home page**: the Mirror tab is gone. When nothing is playing, a small round button with a ring of beams sits on the clock card. Pressing it grows the button into the camera view over the whole home page; the close button shrinks it back to the button while the beams unload. The camera still only runs while the view is open, and stops when you leave the home page or close the notch.

### Added

- **Rolling clock**: the home clock can now show bare digits (seconds included, nearly as bright as the rest) that slide up a good way and fade as the next one rises in from below. It is the new default and costs less than the flip clock (two text draws per moving digit, no tiles). Settings → Look → Home clock switches back to the flip clock; `clock_style = "roll"` or `"flip"` in `[appearance]`.
- **Beam style**: Settings → Look has two switches, "Blue beams, a hint of white" (the default) and "White beams, a hint of blue". Only one can be on at a time. The choice changes every beam in the app, and `white_beams` in `[appearance]` holds it.
- **Choose the hello's font**: set `[greeting] font = "Sacramento"` (any installed handwriting font) and the notch writes the whole greeting, "Hello" and your name, in that font with the same left-to-right write-on and pen beam. Left empty, it uses Yellowtail when that is installed, and otherwise the built-in pen drawing.
- **Handwritten hello on unlock**: once the check mark lands, the scan fades, the notch settles into a slightly smaller stage and a pen writes "hello" in one flowing blue stroke (drawn by the app itself, so it looks the same on every PC). A bright beam with a short trail rides at the pen, and a rule of beams along the bottom fills as it writes.
- **Say hello to your name**: Settings → General has a new "Say hello to" field; with a name set, the name is written below the hello in a handwriting font (Segoe Script or the nearest installed script face). Leave it empty for just the hello.
- **Switch output by hovering**: rest the pointer on the speaker icon of the volume peek for about 0.7 s and the default playback device moves to the next one; a peek names the new output. It does nothing until you hover, and a short script sets the device (Windows has no public call for it).
- **Microphone peek**: a peek when the default microphone is muted or unmuted (from a hardware key, the mixer or Windows settings).
- **USB drive peek**: a short notice when a drive is connected or removed. It runs only on Windows' device message, so nothing is polled.

### Changed

- **Cheaper hello**: the font-written hello measures its text once per greeting instead of twice every frame, and the unlock animation stops asking for frames once the writing is finished (the last second is a still picture).
- **Lighter in the background**: the dev-server scan now starts at every 3 s and backs off to every 10 s while nothing changes (it speeds up again as soon as a server starts or stops). Flip clock digits that are not turning draw as one tile instead of two halves, and the pen-drawn hello uses about a third fewer shapes per frame.
- **Quieter Wi-Fi and Bluetooth center**: the sliding pill is now two plain labels over a hairline with a single blue beam under the open one. The lists lost their Connect/Disconnect/Pair buttons and tinted rows: the whole row is the control, and its action shows as a quiet word on hover ("Connected" in blue on the active network).
- **Flip clock with seconds** on the home page, with or without media playing: six digit tiles that fold over as they change (about 0.6 s per flip), colons between hours, minutes and seconds, and the date shown as "Friday 26th" (no month), with AM/PM beside it on 12-hour clocks. It only updates once a second while the home page is open; the closed-notch clock stays a quiet text clock.
- **Calmer vinyl and lighter on resources**: the record now turns once every 16 seconds (it was every 2.4) and its frames only run while the player card is actually on screen, at 25 per second. Opening another tab, or collapsing the notch, stops the updates completely.
- **Tab slides follow your tab order**: the slide direction now comes from the order of the header tabs (including a custom order), so moving right always slides right and moving left slides left.
- **Vinyl is always a record**: the media player shows a black spinning vinyl with the album cover as its label (the music note label is used only when there is no cover). It spins while music plays and the notch is open.
- **Gemini input without an outline**: the message box and the key field are flat filled pills.
- **Icons redrawn around the beam**: slightly heavier, evenly weighted strokes, and the media, stats, Wi-Fi, volume and keyboard icons are now built from vertical beams (equalizer, load bars, signal bars, level bars, key beams).
- **Smooth tab switching**: changing pages now slides and cross-fades softly (about 0.6 s) in the direction of the tab, instead of cutting instantly. Flipping tabs with the mouse wheel is paced to match.
- **Face ID style unlock**: when the PC unlocks, the notch grows into a small square and plays a blue scan: corner brackets close in, a ring of ticks fills around a blue smiley face (it blinks once), the smiley turns into a check and "Unlocked" (or your greeting) fades in over a tick bar, then it tucks away after about two seconds.
- **Blue ticks everywhere**: every lit tick (volume and brightness HUD, peeks and notifications, rings, sliders, scrubber, battery, loading sweeps) is now the app's blue, with a light-blue leading edge instead of yellow.
- **Gemini, redesigned for real**: a header with the title, a "Context" tick meter and a tick rail that sweeps while Gemini answers; turns are separated by rows of ticks.
- **Wi-Fi and Bluetooth**: a tick strip under each header sweeps while scanning or connecting and otherwise shows signal strength (Wi-Fi) or how many devices are connected (Bluetooth); Wi-Fi rows show signal as a tick gauge.
- **Ticks in the peeks and the closed notch**: every text peek (media, notifications, Wi-Fi, Bluetooth, clipboard and more) has a rail of ticks that drains while it is shown. Caps/Num/Scroll Lock now use the tick bar like the volume HUD, full when on and empty when off. The closed notch separates its widgets with a single tick and shows the battery as a small tick gauge. Clipboard rows show how fresh each copy is and the home card shows how full the history is.
- **Smooth values everywhere**: gauges, rings, sliders, the media scrubber, percentages, the voice level meter, switches and the segmented control now ease to their new value instead of jumping. One shared helper (`Ui::glide`) does it, and it stops requesting frames the moment a value has settled, so it stays light.
- **Ticks in every module**: Stats gets a tick history graph under each ring; Clipboard, Notifications, Shelf and Dev Servers show how full they are as a tick meter; battery peeks are now tick bars like the volume HUD; Wi-Fi signal and Bluetooth battery are ticks; Weather has humidity and wind gauges plus a rain-chance row per day; Mirror has a tick ring around its button and a live indicator; Gemini shows how much of the conversation it still sees; Ask AI's thinking indicator and every loading state use the tick sweep.
- **Ticks everywhere**: the tick-bar from the volume HUD is now the app's shared gauge language. Progress bars, sliders, the media scrubber, timer and battery rings, battery icons and loading states are all drawn from the same fine ticks, with a travelling sweep while something is loading (including the Mirror camera starting up).
- **Gemini redesign**: the chat now matches the rest of the panel. Your messages are white capsules, Gemini's answers are plain text on black, hairlines separate the turns, a tick sweep shows it is thinking, and the blue accent is gone. Same features, same key handling.
- **Smoother open and close**: the live backdrop blur is skipped while the panel is opaque and while it is moving, GPU caches are kept for 30 s instead of 3 s so the next open does not rebuild them, content fades in sooner, springs settle faster with less overshoot, and the compositor is no longer sent unchanged blur settings every frame.
- **Media player layout**: the transport buttons no longer overlap the scrubber; the times sit on either side of the buttons.
- **Overhauled panel look** (inspired by NotchNook and similar notches): a pure black panel (`expanded_opacity` now defaults to 1.0), flat home columns separated by hairlines instead of cards, plain icon tabs with the open one as a white labeled pill, rounder panel corners and album art, and larger transport buttons. Every tab follows suit: list rows (Clipboard, Wi-Fi, Bluetooth, Downloads, Dev Servers, Notifications, Dictation history, Settings) are flat with hairline separators and a soft highlight on hover, and the weather page uses dividers instead of cards. Nothing about what the app does has changed.
- **More Mac-like panel**: the open tab in the header is a white pill with its name, tiles are rounder (16 px) with a faint light edge, and the media scrubber is now glassy: a frosted track, a sheen along the fill and a translucent bead.
- **Unlock-pill style everywhere**: every peek now uses a plain white icon on the left and a small coloured status badge on the right (green check, warning mark, or a dot), in a slimmer, rounder pill. Icon-only peeks are even more compact.
- **Volume and brightness HUD**: icon top left, percentage top right and a bar of thin ticks underneath, with the leading ticks warming to yellow.
- **Unlock greeting** is now just the padlock and a green check. Set `[greeting] name` to get the "Good morning, Sam" text and date back.

- **Tab order is yours**: Settings → Tabs reorders the header tabs with ‹ › arrows (`[tabs] order` in the config). Home stays first.

- Home and notch widgets can be reordered with ‹ › arrows (Settings → Home / Notch).
- Gemini tab redone with very round corners: pill-shaped input, bubble messages and a soft rounded panel.
- The unlock greeting uses a plain white unlocked padlock instead of the sparkle in a coloured circle.

- **New look, inspired by Bloom**: black glass notch with rounder bottom corners and larger concave ears, 12 px tiles with a hairline border, round header buttons where the open tab is a white circle with a dark icon, white "active" controls (selected chips, segmented buttons, primary buttons), thin thumbless sliders, green iOS-style switches, a white play button, and an Inter-first font (used when Inter is installed; otherwise Segoe UI Variable). Expanded opacity default is now 0.92.
- **System stats page**: three big rings (CPU, memory, graphics). Click the stats widget on the home page to open it.

- Scroll the mouse wheel over the header to flip through the tabs.
- The system stats widget shows CPU, RAM and GPU as three rings instead of three bars.

- All icons are now drawn as thin rounded vector outlines in the style of macOS SF Symbols (no icon font needed any more). Media transport buttons stay filled.

### Added

- **Mirror** tab: a small live camera preview that only runs after you press the Mirror button. It switches off when you close it, leave the tab or close the notch (`mirror = false` under `[modules]` removes the tab).
- **Spinning vinyl** instead of a bare music icon when a track has no artwork (the open player turns it at 30 fps; the closed notch shows it still, so playback stays free).

- **Settings** tab: general options, look (glass, opacity, size, bounce, speed, accent), home widgets, module switches, weather city and units, Gemini key and model.
- **Customizable home page**: choose up to four widgets (`[home] widgets`). The clock now stays on the left: a strip under the media player while something plays, the big clock otherwise.
- **Weather** tab and home widget (Open-Meteo, no key).
- **Control Center** tab (Wi-Fi and Bluetooth in one): two buttons at the top slide between the lists. Wi-Fi: scan, connect, disconnect, radio switch. Bluetooth: scan, pair, connect (best effort), disconnect, remove, radio switch. One home widget with a half for each.
- **Notifications**: Windows notifications peek in the notch (needs Windows' notification access), with a short history tab.
- **Gemini** tab with in-app API key entry (saved to `%APPDATA%\DynamicNotch\.env`).
- **Notes** tab and widget.
- **Unlock greeting**: a short "Good morning, Sam" peek with the date when you unlock the PC (`[greeting] name`, switch off in Settings).
- **Disconnect** button for connected Bluetooth devices (keeps them paired; works for headphones, speakers and similar).
- Header tabs shrink when many modules are enabled; the Settings gear sits at the right end of the header, next to the battery.
- More breathing room when expanded: wider default panel (720), larger padding and rows. Panel width is adjustable in Settings → Look.
- Gemini chat shows the whole conversation, with a compact input at the bottom.

## [0.1.0] - 2026-10-08

The first public release.

### Notch

- A MacBook-style notch at the top of the screen that rests, shows live activities, peeks on events and opens into a frosted-glass panel.
- Spring animations, hover lean, fast collapse when the mouse leaves.
- Hides automatically for fullscreen games, videos and presentations.
- Idle styles: always-visible notch, slim pill, or hidden until something happens.
- Primary monitor, a fixed monitor, or follow the active window.
- Per-monitor DPI aware; renders with Direct2D on a composition swap chain and draws nothing while idle.

### Modules

- **Media**: artwork, title and controls for anything using the Windows media controls, with a compositor-driven visualizer tinted from the album art.
- **Timer**: timers and a stopwatch with a countdown ring in the notch and a chime when done.
- **Downloads**: browser downloads with size and speed, and a peek when they finish.
- **Clipboard**: in-memory history of text, links, files and images; content flagged by password managers is skipped.
- **Shelf**: drop files on the notch to park them, drag them out later.
- **Dictation**: hold or tap `Ctrl+Alt+D`, speak, and the text is pasted into the app you were using (Whisper on Groq, with an optional clean-up pass).
- **Ask AI**: a quick prompt on `Ctrl+Alt+Space` with streamed answers and follow-ups, through any OpenRouter model.
- **Mic and camera**: an indicator and a peek naming the app whenever the microphone or camera is in use.
- **Ports**: local dev servers as they start listening, each with an Open button.
- **Lock keys**: peeks for Caps Lock, Num Lock and Scroll Lock.
- **Battery**: level and charging state, with peeks for plugged in, unplugged, low and full.
- **Volume and brightness**: the notch replaces the Windows volume flyout; brightness changes on the built-in display peek too.
- **Bluetooth**: connect and disconnect peeks with the device's battery level, and a low-battery warning.
- **System stats**: CPU, RAM and GPU load on the home page, sampled only while the notch is open.

### Integrations

- Command-line API: `notify`, `progress`, `timer`, `ask`, `toggle`, `expand`, `collapse`, `ai`, `reload` and `quit` are forwarded to the running notch.
- Claude Code hook bridge: `dynamic-notch claude` peeks when Claude finishes or needs you and brings its terminal back on click.
- Tray icon with modules, monitor, display options and autostart.
- `%APPDATA%\DynamicNotch\config.toml` with hot reload; API keys are read only from the environment or a git-ignored `.env`.

[Unreleased]: https://github.com/eigenes/DynamicNotch/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/eigenes/DynamicNotch/releases/tag/v0.1.0
