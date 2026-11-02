//! User configuration: `%APPDATA%\DynamicNotch\config.toml`.
//!
//! Every field has a default, so partial files are fine.  The file is watched
//! and hot-reloaded on save.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::util::app_dir;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub general: General,
    pub appearance: Appearance,
    pub hotkeys: Hotkeys,
    pub modules: Modules,
    pub media: MediaCfg,
    pub timer: TimerCfg,
    pub clipboard: ClipboardCfg,
    pub downloads: DownloadsCfg,
    pub battery: BatteryCfg,
    pub ai: AiCfg,
    pub locks: LocksCfg,
    pub shelf: ShelfCfg,
    pub ports: PortsCfg,
    pub voice: VoiceCfg,
    pub osd: OsdCfg,
    pub bluetooth: BluetoothCfg,
    pub home: HomeCfg,
    pub weather: WeatherCfg,
    pub gemini: GeminiCfg,
    pub greeting: GreetingCfg,
    pub update: UpdateCfg,
    pub notch: NotchCfg,
    pub tabs: TabsCfg,
    pub glass: GlassCfg,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// "primary", "active" (follows the focused window) or a 1-based monitor number.
    pub monitor: String,
    pub expand_on_hover: bool,
    /// Short, quiet tick on hover and click.
    pub haptics: bool,
    /// Alcove: music and finished peeks leave small bubbles beside the notch.
    pub bubbles: bool,
    /// A very quiet blip as a bubble pops out and again as it goes.
    pub bubble_sounds: bool,
    /// Alcove: volume and brightness show as one big round dial (false: the line). Battery and
    /// Bluetooth are always dials.
    pub hud_ring: bool,
    /// Round dials (volume, brightness, battery, Bluetooth) fill clockwise (false: counter-clockwise).
    pub hud_clockwise: bool,
    pub hover_delay_ms: u32,
    pub collapse_delay_ms: u32,
    pub hide_in_fullscreen: bool,
    /// "notch" (always visible), "pill" (thin line when idle) or "hidden" (only on activity/hover).
    pub idle_style: String,
    pub start_with_windows: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            monitor: "primary".into(),
            expand_on_hover: false,
            haptics: true,
            bubbles: true,
            bubble_sounds: false,
            hud_ring: true,
            hud_clockwise: false,
            hover_delay_ms: 140,
            collapse_delay_ms: 60,
            hide_in_fullscreen: true,
            idle_style: "notch".into(),
            start_with_windows: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub notch_width: f32,
    pub notch_height: f32,
    pub expanded_width: f32,
    /// Background opacity when collapsed (1.0 = solid black like a real notch).
    pub collapsed_opacity: f32,
    /// Background opacity when expanded (lower = more glass).
    pub expanded_opacity: f32,
    /// Blur what is behind the expanded notch.
    pub blur: bool,
    pub shadow: bool,
    /// 0 = no overshoot, 1 = very bouncy.
    pub bounce: f32,
    /// Animation speed multiplier (2.0 = twice as fast).
    pub animation_speed: f32,
    /// "auto" (from album art) or "#RRGGBB".
    pub accent: String,
    /// Overall look: "beams" (glowing beams everywhere), "calm" (beams only on volume, brightness
    /// and battery; glow bars, pills and dot rings elsewhere), "minimal" (thin plain lines and
    /// quiet, bounce-free motion) or "nothing" (dot-matrix glyph lights, red accent).
    pub look: String,
    /// false = blue beams with a hint of white (default), true = white beams with a hint of blue.
    pub white_beams: bool,
    /// Home clock: "roll" (digits slide up and are replaced from below) or "flip" (flip-clock tiles).
    pub clock_style: String,
    /// Draw the content once and only move/fade the picture while the notch opens, closes or
    /// switches pages (much less work per frame).
    pub motion_cache: bool,
    /// Alcove: the bottom edge of the open notch softens into a light blur.
    pub bottom_fade: bool,
    /// Alcove: the open notch's edge bends what is behind it, like thick glass.
    pub glass_look: bool,
    /// Alcove: while media is playing and the notch is open, the glass edge keeps up with what is
    /// behind the notch (the screen is looked at again a few times a second). Turn off to save CPU.
    pub glass_live: bool,
    /// Lighter animations: the glass edge is rebuilt less often (and coarser) while the notch moves,
    /// and the slow last stretch of a move uses fewer frames. Looks the same once the notch rests.
    /// Turn off for the full-rate animation.
    pub lite_motion: bool,
    /// Icons are drawn in white or grey whatever colour a page gives them (no coloured tints).
    pub mono_icons: bool,
    /// Most frames a second the notch draws while it moves (30 - 240). 60 is smooth and light; raise it on a fast display for
    /// even smoother motion at a higher CPU cost.
    pub max_fps: f32,
    /// Extra UI scale on top of the monitor DPI.
    pub scale: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            notch_width: 188.0,
            notch_height: 32.0,
            expanded_width: 720.0,
            collapsed_opacity: 1.0,
            expanded_opacity: 1.0,
            blur: true,
            shadow: false,
            bounce: 0.3,
            animation_speed: 1.0,
            accent: "auto".into(),
            look: "alcove".into(),
            white_beams: false,
            clock_style: "roll".into(),
            motion_cache: true,
            bottom_fade: true,
            glass_look: true,
            glass_live: true,
            lite_motion: true,
            mono_icons: true,
            max_fps: 60.0,
            scale: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub toggle: String,
    pub ai: String,
    pub timer: String,
    pub clipboard: String,
    pub play_pause: String,
    /// Voice dictation: tap to start/stop, or hold to talk and release to finish.
    pub dictate: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Alt+N".into(),
            ai: "Ctrl+Alt+Space".into(),
            timer: "Ctrl+Alt+T".into(),
            clipboard: "Ctrl+Alt+V".into(),
            play_pause: String::new(),
            dictate: "Ctrl+Alt+D".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Modules {
    pub media: bool,
    pub timer: bool,
    pub downloads: bool,
    pub privacy: bool,
    pub clipboard: bool,
    pub battery: bool,
    pub ai: bool,
    pub voice: bool,
    pub shelf: bool,
    pub ports: bool,
    pub locks: bool,
    pub osd: bool,
    pub bluetooth: bool,
    pub stats: bool,
    pub weather: bool,
    pub wifi: bool,
    pub gemini: bool,
    pub notes: bool,
    pub greeting: bool,
    pub control: bool,
    pub notifications: bool,
    pub mirror: bool,
    pub devices: bool,
}

impl Default for Modules {
    fn default() -> Self {
        Self {
            media: true,
            timer: true,
            downloads: true,
            privacy: true,
            clipboard: true,
            battery: true,
            ai: true,
            voice: true,
            shelf: true,
            ports: true,
            locks: true,
            osd: true,
            bluetooth: true,
            stats: true,
            weather: true,
            wifi: true,
            gemini: true,
            notes: true,
            greeting: true,
            control: true,
            notifications: true,
            mirror: true,
            devices: true,
        }
    }
}

impl Modules {
    pub fn is_enabled(&self, id: &str) -> bool {
        match id {
            "media" => self.media,
            "timer" => self.timer,
            "downloads" => self.downloads,
            "privacy" => self.privacy,
            "clipboard" => self.clipboard,
            "battery" => self.battery,
            "ai" => self.ai,
            "voice" => self.voice,
            "shelf" => self.shelf,
            "ports" => self.ports,
            "locks" => self.locks,
            "osd" => self.osd,
            "bluetooth" => self.bluetooth,
            "stats" => self.stats,
            "weather" => self.weather,
            "wifi" => self.wifi,
            "gemini" => self.gemini,
            "notes" => self.notes,
            "greeting" => self.greeting,
            "control" => self.control,
            "notifications" => self.notifications,
            "mirror" => self.mirror,
            "devices" => self.devices,
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaCfg {
    pub visualizer: bool,
    /// Show a peek when the track changes.
    pub peek_on_track_change: bool,
    /// Keep the live activity this many seconds after playback pauses.
    pub linger_after_pause_secs: u32,
}

impl Default for MediaCfg {
    fn default() -> Self {
        Self { visualizer: true, peek_on_track_change: true, linger_after_pause_secs: 20 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TimerCfg {
    pub presets_min: Vec<u32>,
    pub sound: bool,
    /// Gain applied to the "done" chime (1.0 = as recorded).
    pub sound_volume: f32,
}

impl Default for TimerCfg {
    fn default() -> Self {
        Self { presets_min: vec![1, 3, 5, 10, 15, 25, 45], sound: true, sound_volume: 1.6 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardCfg {
    pub history: usize,
    pub flash_on_copy: bool,
}

impl Default for ClipboardCfg {
    fn default() -> Self {
        Self { history: 8, flash_on_copy: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadsCfg {
    /// Extra folders to watch (the user's Downloads folder is always watched).
    pub folders: Vec<String>,
    pub peek_on_complete: bool,
}

impl Default for DownloadsCfg {
    fn default() -> Self {
        Self { folders: Vec::new(), peek_on_complete: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryCfg {
    pub low_threshold: u8,
    pub peek_on_plug: bool,
}

impl Default for BatteryCfg {
    fn default() -> Self {
        Self { low_threshold: 20, peek_on_plug: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AiCfg {
    /// Any OpenRouter model id. The API key is never stored here: it is read
    /// from OPENROUTER_API_KEY (environment or a git-ignored `.env` file).
    pub model: String,
    /// OpenAI-compatible chat completions endpoint.
    pub endpoint: String,
    pub max_tokens: u32,
    pub system_prompt: String,
}

impl Default for AiCfg {
    fn default() -> Self {
        Self {
            model: "google/gemini-3.1-flash-lite".into(),
            endpoint: "https://openrouter.ai/api/v1/chat/completions".into(),
            max_tokens: 4096,
            system_prompt: "You are a quick assistant living in a small desktop notch. \
                Answer concisely: prefer a few short sentences or a compact list. Plain text, no markdown headings."
                .into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LocksCfg {
    /// Peek when Caps Lock / Num Lock / Scroll Lock is toggled.
    pub caps: bool,
    pub num: bool,
    pub scroll: bool,
}

impl Default for LocksCfg {
    fn default() -> Self {
        Self { caps: true, num: true, scroll: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ShelfCfg {
    /// Keep parked files across restarts (only the paths are stored).
    pub remember: bool,
}

impl Default for ShelfCfg {
    fn default() -> Self {
        Self { remember: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PortsCfg {
    /// Peek when a dev server starts listening.
    pub peek_on_start: bool,
    /// Always show these ports, whatever process owns them.
    pub extra_ports: Vec<u16>,
    /// Never show these ports.
    pub ignore_ports: Vec<u16>,
    /// Extra process names that count as dev servers (e.g. "myserver.exe").
    pub extra_processes: Vec<String>,
}

impl Default for PortsCfg {
    fn default() -> Self {
        Self { peek_on_start: true, extra_ports: Vec::new(), ignore_ports: Vec::new(), extra_processes: Vec::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceCfg {
    /// Groq speech-to-text model. The API key is never stored here: it is read
    /// from GROQ_API_KEY (environment or a git-ignored `.env` file).
    pub model: String,
    /// OpenAI-compatible API base URL.
    pub endpoint: String,
    /// "" = detect automatically, or an ISO code like "en" / "de".
    pub language: String,
    /// Names and terms the transcription should spell correctly.
    pub vocabulary: Vec<String>,
    /// LLM pass after transcription: "light" (fillers, punctuation),
    /// "medium" (also light rephrasing) or "off".
    pub cleanup: String,
    pub cleanup_model: String,
    /// Paste into the focused app; otherwise the text is only copied.
    pub auto_paste: bool,
    /// Recording stops automatically after this long.
    pub max_seconds: u32,
}

impl Default for VoiceCfg {
    fn default() -> Self {
        Self {
            model: "whisper-large-v3-turbo".into(),
            endpoint: "https://api.groq.com/openai/v1".into(),
            language: String::new(),
            vocabulary: Vec::new(),
            cleanup: "light".into(),
            cleanup_model: "llama-3.1-8b-instant".into(),
            auto_paste: true,
            max_seconds: 300,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdCfg {
    /// Peek when the output volume or mute state changes.
    pub volume: bool,
    /// Handle the volume keys ourselves so Windows' own flyout stays hidden
    /// (falls back to the Windows flyout while the notch is hidden or open).
    pub replace_system_flyout: bool,
    /// Volume change per key press, in percent.
    pub volume_step: u32,
    /// Peek when the built-in display's brightness changes.
    pub brightness: bool,
}

impl Default for OsdCfg {
    fn default() -> Self {
        Self { volume: true, replace_system_flyout: true, volume_step: 2, brightness: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BluetoothCfg {
    pub peek_on_connect: bool,
    pub peek_on_disconnect: bool,
    /// Warn once when a connected device's battery drops to this percentage.
    pub low_battery: u8,
}

impl Default for BluetoothCfg {
    fn default() -> Self {
        Self { peek_on_connect: true, peek_on_disconnect: true, low_battery: 15 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeCfg {
    /// Up to four widgets for the right-hand column of the home page, top to bottom.
    /// Ids: weather, stats, timer, control (Wi-Fi & Bluetooth), notes, clipboard, downloads, shelf, ports.
    pub widgets: Vec<String>,
}

impl Default for HomeCfg {
    fn default() -> Self {
        Self { widgets: ["weather", "stats", "timer", "control"].iter().map(|s| s.to_string()).collect() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherCfg {
    /// City to show, e.g. "Berlin" or "Springfield, IL". Empty = not set up yet.
    pub city: String,
    /// "metric" (°C, km/h) or "imperial" (°F, mph).
    pub units: String,
    /// Minimum minutes between automatic refreshes.
    pub refresh_minutes: u32,
}

impl Default for WeatherCfg {
    fn default() -> Self {
        Self { city: String::new(), units: "metric".into(), refresh_minutes: 20 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GeminiCfg {
    /// The API key is never stored here: paste it in Settings → Gemini, or put
    /// GEMINI_API_KEY in a .env file.
    pub model: String,
    pub endpoint: String,
    pub max_tokens: u32,
    pub system_prompt: String,
}

impl Default for GeminiCfg {
    fn default() -> Self {
        Self {
            model: "gemini-2.5-flash".into(),
            endpoint: "https://generativelanguage.googleapis.com/v1beta".into(),
            max_tokens: 2048,
            system_prompt: "You are a helpful assistant living in a small desktop notch. \
                Keep answers short and clear. Plain text, no markdown headings."
                .into(),
        }
    }
}

/// Where "Check for updates" (Settings → General) looks for new releases.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateCfg {
    /// GitHub `owner/name` whose newest release is offered.
    pub repo: String,
}

impl Default for UpdateCfg {
    fn default() -> Self {
        Self { repo: "TonyB1119-arch/Notch".into() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GreetingCfg {
    /// Name to say hello to ("Hello Sam"). Empty = just "Hello".
    pub name: String,
    /// Name of an installed handwriting font for the hello (e.g. "Sacramento"). Empty = Yellowtail when installed, else the built-in pen drawing.
    pub font: String,
}

impl GreetingCfg {
    /// The font to write the hello with: the configured one, or Yellowtail when nothing is set
    /// (used only if it is installed; otherwise the built-in pen drawing is used).
    pub fn font_name(&self) -> &str {
        let f = self.font.trim();
        if f.is_empty() {
            "Yellowtail"
        } else {
            f
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NotchCfg {
    /// Up to three small widgets on the closed notch: clock, date, battery, weather, wifi, bluetooth, notes.
    pub widgets: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TabsCfg {
    /// Order of the tabs in the header (module ids). Home always comes first;
    /// tabs not listed follow in the default order. Change it in Settings → Tabs.
    pub order: Vec<String>,
}

/// The look of the Alcove glass edge (Settings → Glass). Sizes are in logical pixels.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GlassCfg {
    /// How far in from the edge the glass reaches.
    pub edge_width: f32,
    /// How far out the glass looks at its strongest (the warp).
    pub pull: f32,
    /// How sharply the bend is concentrated at the very edge (1 = even, 3 = only right at the edge).
    pub bend: f32,
    /// How solid the glass is at the edge (0.3 - 1).
    pub strength: f32,
    /// Extra warp in the rounded corners (0 - 1).
    pub corner: f32,
    /// Colour fringing: red, green and blue look slightly different distances out.
    pub fringe: f32,
    /// How far up the sides the glass is drawn before it fades out.
    pub reach: f32,
    /// How softly the glass starts at the outer edge.
    pub softness: f32,
    /// Glass across the middle of the notch, wherever it is even a little see-through (0 = off).
    pub middle: f32,
    /// How much the middle glass bends what is behind it (0 = clear, 1 = like the edge).
    pub middle_warp: f32,
    /// How much the middle glass softens what is behind it, in logical pixels (0 = sharp).
    pub middle_blur: f32,
    /// How see-through the bottom edge of the notch gets (0 = not at all).
    pub fade_amount: f32,
    /// How high up the bottom fade begins (1 = the usual height).
    pub fade_start: f32,
    /// How quickly the glass fades in (higher = quicker).
    pub glow_in: f32,
    /// Live glass: milliseconds between looks at the screen while media plays.
    pub live_ms: f32,
    /// Live glass: seconds for the edge to melt from one picture into the next.
    pub melt: f32,
    /// While the notch moves, the glass gets its finest detail once this fraction of the move is left.
    pub sharpen_at: f32,
    /// How far in from the edge the glass is worked out pixel for pixel, in edge widths (4 = everywhere).
    pub sharp_zone: f32,
    /// Stretch the glass edge like iOS Liquid Glass: the picture under the glass is pulled out toward
    /// the edge (it can fold over right at the rim) instead of the picture outside being pulled in.
    pub stretch: bool,
    /// A thin bright line along the very edge of the glass, like the light catching it (0 - 1).
    pub edge_light: f32,
    /// A light frost over the glass (0 - 1).
    pub tint: f32,
    /// Stretch mode: how far the stretch leans sideways toward the ends of the bottom edge (radians).
    pub fan: f32,
    /// Stretch mode: how long the stretch may be, relative to the longest one that never folds over.
    pub stretch_len: f32,
    /// How much the glass dims when the picture behind it is bright, so white text stays clear (0 - 1).
    pub readable: f32,
}

impl Default for GlassCfg {
    fn default() -> Self {
        Self {
            edge_width: 76.0,
            pull: 22.0,
            bend: 1.5,
            strength: 0.88,
            corner: 0.3,
            fringe: 0.05,
            reach: 90.0,
            softness: 4.0,
            middle: 0.85,
            middle_warp: 0.0,
            middle_blur: 0.0,
            fade_amount: 0.8,
            fade_start: 1.7,
            glow_in: 9.0,
            live_ms: 150.0,
            melt: 0.12,
            sharpen_at: 0.12,
            sharp_zone: 4.0,
            stretch: false,
            edge_light: 0.35,
            tint: 0.0,
            fan: 0.7,
            stretch_len: 1.0,
            readable: 0.4,
        }
    }
}

impl GlassCfg {
    fn clamped(&mut self) {
        self.edge_width = self.edge_width.clamp(20.0, 160.0);
        self.pull = self.pull.clamp(0.0, 60.0);
        self.bend = self.bend.clamp(0.8, 3.0);
        self.strength = self.strength.clamp(0.3, 1.0);
        self.corner = self.corner.clamp(0.0, 1.0);
        self.fringe = self.fringe.clamp(0.0, 0.2);
        self.reach = self.reach.clamp(30.0, 200.0);
        self.softness = self.softness.clamp(0.5, 12.0);
        self.middle = self.middle.clamp(0.0, 1.0);
        self.middle_warp = self.middle_warp.clamp(0.0, 1.0);
        self.middle_blur = self.middle_blur.clamp(0.0, 6.0);
        self.fade_amount = self.fade_amount.clamp(0.0, 1.0);
        self.fade_start = self.fade_start.clamp(0.5, 3.0);
        self.glow_in = self.glow_in.clamp(2.0, 30.0);
        self.live_ms = self.live_ms.clamp(80.0, 500.0);
        self.melt = self.melt.clamp(0.03, 0.5);
        self.sharpen_at = self.sharpen_at.clamp(0.02, 0.5);
        self.sharp_zone = self.sharp_zone.clamp(0.3, 4.0);
        self.edge_light = self.edge_light.clamp(0.0, 1.0);
        self.tint = self.tint.clamp(0.0, 1.0);
        self.fan = self.fan.clamp(0.0, 1.2);
        self.stretch_len = self.stretch_len.clamp(0.5, 1.6);
        self.readable = self.readable.clamp(0.0, 1.0);
    }
}

pub fn config_path() -> PathBuf {
    app_dir().join("config.toml")
}

// returned once per load and unpacked right away; boxing would buy nothing
#[allow(clippy::large_enum_variant)]
pub enum LoadResult {
    Ok(Config),
    /// File had errors; defaults (or last good config) should be kept.
    Invalid(String),
}

pub fn load() -> LoadResult {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(c) => LoadResult::Ok(c.sanitized()),
            Err(e) => LoadResult::Invalid(e.message().to_string()),
        },
        Err(_) => {
            let _ = std::fs::write(&path, DEFAULT_TEMPLATE);
            LoadResult::Ok(Config::default())
        }
    }
}

/// Persist a single `[section] key = value` change while keeping the user's
/// comments and formatting intact.
pub fn set_value(section: &str, key: &str, value: &str) {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_TEMPLATE.to_string());
    let mut out = Vec::new();
    let mut in_section = false;
    let mut done = false;
    let mut section_seen = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            if in_section && !done {
                out.push(format!("{key} = {value}"));
                done = true;
            }
            in_section = &t[1..t.len() - 1] == section;
            section_seen |= in_section;
        } else if in_section && !done {
            let k = t.split('=').next().unwrap_or("").trim();
            if k == key && !t.starts_with('#') {
                out.push(format!("{key} = {value}"));
                done = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    if !done {
        if !section_seen {
            out.push(String::new());
            out.push(format!("[{section}]"));
        }
        out.push(format!("{key} = {value}"));
    }
    let _ = std::fs::write(&path, out.join("\n") + "\n");
}

impl Config {
    fn sanitized(mut self) -> Self {
        let a = &mut self.appearance;
        a.notch_width = a.notch_width.clamp(80.0, 600.0);
        a.notch_height = a.notch_height.clamp(18.0, 64.0);
        a.expanded_width = a.expanded_width.clamp(420.0, 1000.0);
        a.collapsed_opacity = a.collapsed_opacity.clamp(0.2, 1.0);
        a.expanded_opacity = a.expanded_opacity.clamp(0.2, 1.0);
        a.bounce = a.bounce.clamp(0.0, 1.0);
        a.animation_speed = a.animation_speed.clamp(0.25, 4.0);
        a.scale = a.scale.clamp(0.5, 2.5);
        a.max_fps = a.max_fps.clamp(30.0, 240.0);
        self.clipboard.history = self.clipboard.history.clamp(1, 30);
        self.voice.max_seconds = self.voice.max_seconds.clamp(5, 780);
        self.osd.volume_step = self.osd.volume_step.clamp(1, 25);
        self.bluetooth.low_battery = self.bluetooth.low_battery.min(100);
        self.weather.refresh_minutes = self.weather.refresh_minutes.clamp(5, 360);
        self.glass.clamped();
        self.home.widgets.truncate(4);
        self.notch.widgets.truncate(3);
        self
    }
}

pub const DEFAULT_TEMPLATE: &str = r##"# Dynamic Notch configuration
# Saved changes are applied immediately (hot reload).

[general]
# "primary", "active" (follow the focused window) or a monitor number (1, 2, ...)
monitor = "primary"
# hovering only grows the notch slightly; a click opens it (set true to open on hover)
expand_on_hover = false
# short, quiet tick on hover and click
haptics = true
# Alcove: music and finished peeks leave small bubbles beside the notch
bubbles = true
# a very quiet blip as a bubble pops out and again as it goes
bubble_sounds = false
# Alcove: volume and brightness as one big round dial (false: the line)
hud_ring = true
# round dials fill clockwise (false: counter-clockwise)
hud_clockwise = false
hover_delay_ms = 140
collapse_delay_ms = 60
hide_in_fullscreen = true
# "notch" = always visible, "pill" = slim line when idle, "hidden" = only on activity/hover
idle_style = "notch"
start_with_windows = false

[appearance]
notch_width = 188
notch_height = 32
expanded_width = 720
collapsed_opacity = 1.0
expanded_opacity = 1.0
blur = true
shadow = false
# 0 = no overshoot ... 1 = very bouncy
bounce = 0.3
animation_speed = 1.0
# "alcove" (island layouts, clock and weather home, page dots), "beams" (beams everywhere),
# "calm" (beams only on volume, brightness and battery), "minimal" (thin plain lines and quiet motion),
# "nothing" (dot-matrix glyph lights, red accent) or "bloom" (capsule gauges, control center)
look = "alcove"
# false = blue beams with a hint of white, true = white beams with a hint of blue
white_beams = false
# "roll" (digits slide up, the next one rises in) or "flip" (flip-clock tiles)
clock_style = "roll"
# Draw content once and only move/fade it while opening, closing and switching pages
motion_cache = true
# Alcove: the bottom edge of the open notch softens into a light blur
bottom_fade = true
# Alcove: the notch's edge bends what is behind it (false = plain glass)
glass_look = true
# Alcove: while media plays and the notch is open, the glass edge keeps up with what is behind it
# (a little extra CPU; during that time the notch is hidden from screenshots and screen recordings)
glass_live = true
# Lighter animations: less CPU while the notch moves (the glass edge is refreshed less often, the slow
# end of a move uses fewer frames). false = the full-rate animation
lite_motion = true
# Monochrome icons: every icon is white or grey, no coloured tints or halos. false = coloured icons
mono_icons = true
# Most frames a second while the notch moves (30 - 240). 60 is smooth and light; raise it on a fast display
# for smoother motion at a higher CPU cost.
max_fps = 60
# "auto" picks the accent from album art, or use "#RRGGBB"
accent = "auto"
scale = 1.0

[hotkeys]
# Modifiers: Ctrl, Alt, Shift, Win. Keys: A-Z, 0-9, F1-F24, Space, Enter, Tab, Esc,
# Up/Down/Left/Right, Home, End, PageUp, PageDown, Insert, Delete, `, -, =, [, ], ;, ', ",", ., /
# Leave empty to disable.
toggle = "Ctrl+Alt+N"
ai = "Ctrl+Alt+Space"
timer = "Ctrl+Alt+T"
clipboard = "Ctrl+Alt+V"
play_pause = ""
# Voice dictation: tap to start/stop, or hold to talk and release to finish.
dictate = "Ctrl+Alt+D"

[modules]
media = true
timer = true
downloads = true
privacy = true
clipboard = true
battery = true
ai = true
voice = true
shelf = true
ports = true
locks = true
osd = true
stats = true
weather = true
control = true            # Wi-Fi & Bluetooth in one tab
notifications = true      # Windows notifications in the notch
mirror = true             # Mirror tab (the camera only runs while you press its button)
devices = true            # peeks when a USB drive or a display is connected
gemini = true
notes = true
# greet you when you unlock the PC
greeting = true

[media]
visualizer = true
peek_on_track_change = true
linger_after_pause_secs = 20

[timer]
presets_min = [1, 3, 5, 10, 15, 25, 45]
sound = true
# loudness of the "done" chime; 1.0 = system default, capped before it would clip
sound_volume = 1.6

[clipboard]
history = 8
flash_on_copy = true

[downloads]
# Extra folders to watch; your Downloads folder is always watched.
folders = []
peek_on_complete = true

[battery]
low_threshold = 20
peek_on_plug = true

[ai]
# The OpenRouter API key is NOT stored here. Put it in a .env file:
#   OPENROUTER_API_KEY=sk-or-...
# (searched next to the exe, in the project folder and in %APPDATA%\DynamicNotch)
# Any OpenRouter model id. Cheap & fast default; alternatives:
# "deepseek/deepseek-v4.1-flash" (cheapest), "anthropic/claude-haiku-4.5", "anthropic/claude-sonnet-5.5"
model = "google/gemini-3.1-flash-lite"
endpoint = "https://openrouter.ai/api/v1/chat/completions"
max_tokens = 4096
system_prompt = "You are a quick assistant living in a small desktop notch. Answer concisely: prefer a few short sentences or a compact list. Plain text, no markdown headings."

[voice]
# Speech to text with Groq. The API key is NOT stored here. Put it in a .env file:
#   GROQ_API_KEY=gsk_...
# (same places as OPENROUTER_API_KEY). Get one at https://console.groq.com/keys
model = "whisper-large-v3-turbo"
endpoint = "https://api.groq.com/openai/v1"
# "" = detect the language automatically, or e.g. "en", "de"
language = ""
# Names and terms to spell correctly, e.g. ["Groq", "DynamicNotch"]
vocabulary = []
# Clean-up pass after transcription: "light" (fillers, punctuation), "medium" (also light rephrasing) or "off"
cleanup = "light"
cleanup_model = "llama-3.1-8b-instant"
# Paste into the focused app; when false the text is only copied
auto_paste = true
max_seconds = 300

[locks]
# Peek when a lock key is toggled
caps = true
num = true
scroll = false

[shelf]
# Keep parked files across restarts (only the paths are stored)
remember = true

[ports]
# Peek when a dev server starts listening
peek_on_start = true
# Always show / never show these ports
extra_ports = []
ignore_ports = []
# Extra process names that count as dev servers, e.g. ["myserver.exe"]
extra_processes = []

[osd]
# Volume / brightness peeks
volume = true
# Take over the volume keys so the Windows flyout stays hidden
# (the Windows flyout still shows while the notch is hidden or open)
replace_system_flyout = true
# Percent per volume key press
volume_step = 2
# Built-in display only; Windows may still show its own brightness flyout
brightness = true

[bluetooth]
peek_on_connect = true
peek_on_disconnect = true
# Warn once when a connected device's battery drops to this percentage (0 = off)
low_battery = 15

[home]
# Widgets on the right of the home page (max 4, top to bottom). You can also pick them in Settings → Home.
# weather, stats, timer, control (Wi-Fi & Bluetooth), notes, clipboard, downloads, shelf, ports
widgets = ["weather", "stats", "timer", "control"]

[weather]
# Weather comes from open-meteo.com (no account or key). Example: "Berlin" or "Springfield, IL"
city = ""
# "metric" (°C, km/h) or "imperial" (°F, mph)
units = "metric"
refresh_minutes = 20

[gemini]
# Your Gemini key is NOT stored here: paste it in Settings → Gemini, or add
#   GEMINI_API_KEY=...
# to a .env file. Get one at https://aistudio.google.com/apikey
model = "gemini-2.5-flash"
endpoint = "https://generativelanguage.googleapis.com/v1beta"
max_tokens = 2048
system_prompt = "You are a helpful assistant living in a small desktop notch. Keep answers short and clear. Plain text, no markdown headings."

[update]
# "Check for updates" in Settings → General looks at the newest release of this GitHub repo (owner/name).
# Nothing is checked unless you press that button.
repo = "TonyB1119-arch/Notch"

[greeting]
# On unlock the notch scans, then writes "Hello" in handwriting (switch it off with greeting = false under [modules]).
# Name to add after it ("Hello Sam"); empty = just "Hello". Also in Settings → General.
name = ""
# Optional: the name of an installed handwriting font to write the hello with (e.g. "Sacramento", "Yellowtail").
# Empty = Yellowtail if it is installed, otherwise the built-in pen drawing. Also in Settings → General ("Script Regular" needs that font installed).
font = ""

[glass]
# The Alcove glass edge (Settings → Glass). Sizes are in logical pixels; delete a line to get its default.
edge_width = 76.0
pull = 22.0
bend = 1.5
strength = 0.88
corner = 0.3
fringe = 0.05
reach = 90.0
softness = 4.0
middle = 0.85
middle_warp = 0.0
middle_blur = 0.0
fade_amount = 0.8
fade_start = 1.7
glow_in = 9.0
live_ms = 150.0
melt = 0.12
sharpen_at = 0.12
sharp_zone = 4.0
# true = the glass edge stretches the picture under it toward the edge (iOS Liquid Glass look);
# false = it pulls in the picture from outside the notch (the original look)
stretch = false
# a thin bright line along the very edge of the glass (0 - 1)
edge_light = 0.35
# a light frost over the glass (0 - 1)
tint = 0.0
# stretch mode: how far the stretch leans sideways toward the ends of the bottom edge (0 - 1.2)
fan = 0.7
# stretch mode: how long the stretch may be (1 = the longest that never folds over, up to 1.6)
stretch_len = 1.0
# the glass dims a little when what is behind it is bright, so white text stays clear (0 - 1)
readable = 0.4

[tabs]
# Order of the header tabs (Settings → Tabs). Empty = default order.
# ids: timer, stats, voice, clipboard, downloads, shelf, ports, ai, gemini, weather, control, notes, notifications, mirror
order = []

[notch]
# Up to 3 small widgets on the closed notch (Settings → Notch):
# clock, date, battery, weather, wifi, bluetooth, notes
widgets = []

"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_defaults() {
        let c: Config = toml::from_str(DEFAULT_TEMPLATE).expect("template must parse");
        let d = Config::default();
        assert_eq!(c.general.monitor, d.general.monitor);
        assert_eq!(c.appearance.notch_width, d.appearance.notch_width);
        assert_eq!(c.hotkeys.ai, d.hotkeys.ai);
        assert_eq!(c.ai.model, d.ai.model);
        assert_eq!(c.timer.presets_min, d.timer.presets_min);
        assert_eq!(c.hotkeys.dictate, d.hotkeys.dictate);
        assert_eq!(c.voice.model, d.voice.model);
        assert_eq!(c.voice.cleanup, d.voice.cleanup);
        assert_eq!(c.locks.caps, d.locks.caps);
        assert_eq!(c.ports.peek_on_start, d.ports.peek_on_start);
        assert!(c.modules.voice && c.modules.shelf && c.modules.ports && c.modules.locks);
        assert!(c.modules.osd && c.modules.bluetooth && c.modules.stats);
        assert_eq!(c.osd.volume_step, d.osd.volume_step);
        assert_eq!(c.osd.replace_system_flyout, d.osd.replace_system_flyout);
        assert_eq!(c.bluetooth.low_battery, d.bluetooth.low_battery);
        assert_eq!(c.home.widgets, d.home.widgets);
        assert_eq!(c.weather.units, d.weather.units);
        assert_eq!(c.gemini.model, d.gemini.model);
        assert!(
            c.modules.weather
                && c.modules.control
                && c.modules.notifications
                && c.modules.gemini
                && c.modules.notes
                && c.modules.greeting
        );
        assert_eq!(c.greeting.name, d.greeting.name);
        assert_eq!(c.notch.widgets, d.notch.widgets);
        assert_eq!(c.tabs.order, d.tabs.order);
    }

    #[test]
    fn partial_config() {
        let c: Config = toml::from_str("[appearance]\nblur = false\n").unwrap();
        assert!(!c.appearance.blur);
        assert_eq!(c.general.monitor, "primary");
    }
}
