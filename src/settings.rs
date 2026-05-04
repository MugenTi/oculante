use crate::{
    file_encoder::FileEncoder,
    shortcuts::*,
//     utils::ColorChannel
};
// use notan::egui::{Context, Visuals};
use anyhow::{anyhow, Result};
use log::{debug, info, trace};
use serde::{Deserialize, Serialize};
use slint::{LogicalPosition, LogicalSize};

#[cfg(feature = "heif")]
use libheif_rs::SecurityLimits;

use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    fmt::{self, Display, Formatter},
    fs::{create_dir_all, File},
    path::PathBuf,
};

#[cfg(feature = "heif")]
use std::sync::OnceLock;

fn get_config_dir() -> Result<PathBuf> {
    // This uses dirs_next instead of dirs to avoid a dependency conflict for now.
    Ok(dirs_next::data_local_dir()
        .ok_or_else(|| anyhow!("Can't get local dir"))?
        .join("Voutil"))
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum ColorTheme {
    Light,
    Dark,
    System,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct PersistentSettings {
    pub accent_color: [u8; 3],
    pub background_color: [u8; 3],
    pub vsync: bool,
    pub force_redraw: bool,
    pub shortcuts: Shortcuts,
    pub keep_view: bool,
    pub max_cache: usize,
    pub max_recents: u8,
    pub show_scrub_bar: bool,
    pub wrap_folder: bool,
    pub keep_edits: bool,
    pub title_format: String,
    pub info_enabled: bool,
    pub edit_enabled: bool,
    pub show_checker_background: bool,
    pub show_minimap: bool,
    pub show_frame: bool,
    // pub current_channel: ColorChannel,
    pub svg_scale: f32,
    pub zen_mode: bool,
    pub theme: ColorTheme,
    pub linear_mag_filter: bool,
    pub linear_min_filter: bool,
    pub use_mipmaps: bool,
    pub fit_image_on_window_resize: bool,
    pub zoom_multiplier: f32,
    pub auto_scale: bool,
    pub borderless: bool,
    pub min_window_size: (u32, u32),
    pub experimental_features: bool,
    pub decoders: DecoderSettings,
    pub show_status_bar: bool,
    pub show_status_messages: bool,
    pub zen_mode_normal: bool,
    pub pan_speed_multiplier: f32,
    pub reopen_last_image: bool,
    pub use_os_sorting: bool,
    pub sort_criteria: String,
    pub sort_order: String,
    pub crop_aspect_ratio: String,
    pub default_save_format: String,
    pub jpeg_quality: u32,
}

impl Default for PersistentSettings {
    fn default() -> Self {
        PersistentSettings {
            accent_color: [255, 0, 75],
            background_color: [30, 30, 30],
            vsync: true,
            force_redraw: false,
            shortcuts: Shortcuts::default_keys(),
            keep_view: Default::default(),
            max_cache: 30,
            max_recents: 12,
            show_scrub_bar: Default::default(),
            wrap_folder: true,
            keep_edits: Default::default(),
            title_format: "{APP} | {VERSION} | {FULLPATH}".into(),
            info_enabled: Default::default(),
            edit_enabled: Default::default(),
            show_checker_background: Default::default(),
            show_minimap: Default::default(),
            show_frame: Default::default(),
            // current_channel: ColorChannel::Rgba,
            svg_scale: 1.0,
            zen_mode: false,
            theme: ColorTheme::Dark,
            linear_mag_filter: false,
            linear_min_filter: true,
            use_mipmaps: true,
            fit_image_on_window_resize: false,
            zoom_multiplier: 1.0,
            auto_scale: false,
            borderless: false,
            min_window_size: (100, 100),
            experimental_features: false,
            decoders: Default::default(),
            show_status_bar: true,
            show_status_messages: true,
            zen_mode_normal: false,
            pan_speed_multiplier: 1.0,
            reopen_last_image: true,
            use_os_sorting: true,
            sort_criteria: "Name".into(),
            sort_order: "Ascending".into(),
            crop_aspect_ratio: "Free".into(),
            default_save_format: "Png".into(),
            jpeg_quality: 85,
        }
    }
}

impl PersistentSettings {
    pub fn load() -> Result<Self> {
        let config_path = get_config_dir()?.join("config.json");
        debug!(
            "Loading persistent settings from: {}",
            config_path.display()
        );
        let file = File::open(config_path)?;
        Ok(serde_json::from_reader(file)?)
    }

    pub fn save_blocking(&self) -> Result<()> {
        let config_dir = get_config_dir()?;
        if !config_dir.exists() {
            create_dir_all(&config_dir)?;
        }
        let config_path = config_dir.join("config.json");
        let f = File::create(&config_path)?;
        serde_json::to_writer_pretty(f, self)?;
        debug!("Saved persistent settings to: {}", config_path.display());
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WindowLayout {
    pub window_position: LogicalPosition,
    pub window_size: LogicalSize,
    pub thumbnail_window_position: LogicalPosition,
    pub thumbnail_window_size: LogicalSize,
    pub thumbnail_window_visible: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct VolatileSettings {
    pub favourite_images: HashSet<PathBuf>,
    pub recent_images: VecDeque<PathBuf>,
    pub window_geometry: ((u32, u32), (u32, u32)), // No use
    pub window_position: LogicalPosition,
    pub window_size: LogicalSize,
    pub thumbnail_window_position: LogicalPosition,
    pub thumbnail_window_size: LogicalSize,
    pub thumbnail_window_visible: bool,
    pub layouts: Vec<Option<WindowLayout>>,
    pub last_open_directory: PathBuf,
    pub folder_bookmarks: BTreeSet<PathBuf>,
    pub image_scale: f64,
    pub last_image_path: PathBuf,
    pub encoding_options: Vec<FileEncoder>,
}

impl Default for VolatileSettings {
    fn default() -> Self {
        Self {
            favourite_images: Default::default(),
            recent_images: Default::default(),
            window_geometry: Default::default(),
            window_position: LogicalPosition {
                x: 100.0,
                y: 100.0,
            },
            window_size: LogicalSize {
                width: 1280.0,
                height: 720.0,
            },
            thumbnail_window_position: LogicalPosition {
                x: 150.0,
                y: 150.0,
            },
            thumbnail_window_size: LogicalSize {
                width: 360.0,
                height: 600.0,
            },
            thumbnail_window_visible: false,
            layouts: vec![None; 3],
            last_open_directory: Default::default(),
            folder_bookmarks: Default::default(),
            image_scale: 1.0,
            last_image_path: Default::default(),
            encoding_options: [
                FileEncoder::Jpg { quality: 75 },
                FileEncoder::WebP,
                FileEncoder::Png {
                    compressionlevel: crate::file_encoder::CompressionLevel::Default,
                },
                FileEncoder::Bmp,
            ]
            .into_iter()
            .collect(),
        }
    }
}

impl VolatileSettings {
    pub fn load() -> Result<Self> {
        let config_path = get_config_dir()?
            .join("config_volatile.json")
            .canonicalize()
            // migrate old config
            ?;

        let s = serde_json::from_reader::<_, VolatileSettings>(File::open(config_path)?)?;
        info!("Loaded volatile settings.");
        Ok(s)
    }

    pub fn save_blocking(&self) -> Result<()> {
        let local_dir = get_config_dir()?;
        if !local_dir.exists() {
            _ = create_dir_all(&local_dir);
        }

        let f = File::create(local_dir.join("config_volatile.json"))?;
        serde_json::to_writer_pretty(f, self)?;
        trace!("Saved volatile settings");
        Ok(())
    }
}

// pub fn set_system_theme(ctx: &Context) {
//     if let Ok(mode) = dark_light::detect() {
//         match mode {
//             dark_light::Mode::Dark => ctx.set_visuals(Visuals::dark()),
//             dark_light::Mode::Light => ctx.set_visuals(Visuals::light()),
//             dark_light::Mode::Unspecified => ctx.set_visuals(Visuals::dark()),
//         }
//     }
// }

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct DecoderSettings {
    /// Settings for libheif
    pub heif: HeifLimits,
}

/// Security limits for HEIF via libheif.
///
/// This is essentially a wrapper for [`SecurityLimits`] to support de/serialization
/// while still working if Voutil is built without libheif support.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct HeifLimits {
    pub image_size_pixels: Limit,
    pub number_of_tiles: Limit,
    pub bayer_pattern_pixels: Limit,
    pub items: Limit,
    pub color_profile_size: Limit,
    pub memory_block_size: Limit,
    pub components: Limit,
    pub iloc_extents_per_item: Limit,
    pub size_entity_group: Limit,
    pub children_per_box: Limit,
    pub override_all: bool,
}

#[cfg(feature = "heif")]
impl From<HeifLimits> for SecurityLimits {
    fn from(limits: HeifLimits) -> Self {
        let mut context = SecurityLimits::new();

        match limits.image_size_pixels {
            Limit::NoLimit => context.set_max_image_size_pixels(0),
            Limit::U64(max) => context.set_max_image_size_pixels(max),
            _ => (),
        }

        match limits.number_of_tiles {
            Limit::NoLimit => context.set_max_number_of_tiles(0),
            Limit::U64(max) => context.set_max_number_of_tiles(max),
            _ => (),
        }

        match limits.bayer_pattern_pixels {
            Limit::NoLimit => context.set_max_bayer_pattern_pixels(0),
            Limit::U32(max) => context.set_max_bayer_pattern_pixels(max),
            _ => (),
        }

        match limits.items {
            Limit::NoLimit => context.set_max_items(0),
            Limit::U32(max) => context.set_max_items(max),
            _ => (),
        }

        match limits.color_profile_size {
            Limit::NoLimit => context.set_max_color_profile_size(0),
            Limit::U32(max) => context.set_max_color_profile_size(max),
            _ => (),
        }

        match limits.memory_block_size {
            Limit::NoLimit => context.set_max_memory_block_size(0),
            Limit::U64(max) => context.set_max_memory_block_size(max),
            _ => (),
        }

        match limits.components {
            Limit::NoLimit => context.set_max_components(0),
            Limit::U32(max) => context.set_max_components(max),
            _ => (),
        }

        match limits.iloc_extents_per_item {
            Limit::NoLimit => context.set_max_iloc_extents_per_item(0),
            Limit::U32(max) => context.set_max_iloc_extents_per_item(max),
            _ => (),
        }

        match limits.size_entity_group {
            Limit::NoLimit => context.set_max_size_entity_group(0),
            Limit::U32(max) => context.set_max_size_entity_group(max),
            _ => (),
        }

        match limits.children_per_box {
            Limit::NoLimit => context.set_max_children_per_box(0),
            Limit::U32(max) => context.set_max_children_per_box(max),
            _ => (),
        }

        context
    }
}

impl HeifLimits {
    /// Return [`SecurityLimits`] if not overridden by LIBHEIF_SECURITY_LIMITS or the settings.
    #[cfg(feature = "heif")]
    pub fn maybe_limits(self) -> Option<SecurityLimits> {
        static OVERRIDE_ALL: OnceLock<bool> = OnceLock::new();

        (!OVERRIDE_ALL.get_or_init(|| {
            // Override settings if the var was set by the time this function is called or use the
            // setting if undefined.
            let override_all = std::env::var("LIBHEIF_SECURITY_LIMITS")
                .ok()
                .as_deref()
                .map(|var| var.eq_ignore_ascii_case("on"))
                .unwrap_or(self.override_all);
            std::env::set_var(
                "LIBHEIF_SECURITY_LIMITS",
                if override_all { "off" } else { "on" },
            );

            override_all
        }))
        .then(|| self.into())
    }
}

/// Limit to specifically store in the config.
///
/// The default values of [`SecurityLimits`] can only be fetched from libheif itself. Therefore, we
/// need a way to store preferences regardless if the `heif` feature is enabled.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub enum Limit {
    #[default]
    Default,
    NoLimit,
    U64(u64),
    U32(u32),
}

impl Display for Limit {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Default => write!(f, ""),
            Self::NoLimit => write!(f, "0"),
            Self::U64(v) => write!(f, "{v}"),
            Self::U32(v) => write!(f, "{v}"),
        }
    }
}
