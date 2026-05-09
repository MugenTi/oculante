#![windows_subsystem = "windows"]
//#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use arboard::{Clipboard, ImageData};
use image::{imageops, DynamicImage, ImageBuffer};
use voutil::file_encoder::{CompressionLevel, FileEncoder};
use voutil::settings::{PersistentSettings, VolatileSettings};
use voutil::shortcuts::{InputEvent, lookup};
use voutil::utils::{apply_color_corrections, reveal_in_file_manager};
use rayon::prelude::*;
use rfd;
use slint::{
    ComponentHandle, Image, LogicalPosition, LogicalSize, Model, Rgba8Pixel,
    SharedPixelBuffer, VecModel,
};
use std::borrow::Cow;
use std::cell::RefCell;
//use std::cmp::min;
use std::collections::HashMap; // Added for sorting
use std::env;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

slint::include_modules!();

mod cache;

#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct SelectionRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl SelectionRect {
    fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    fn get_resize_handle(&self, px: u32, py: u32, tolerance: u32) -> Option<ResizeHandle> {
        let on_left = (px as i32 - self.x as i32).abs() < tolerance as i32;
        let on_right = (px as i32 - (self.x + self.w) as i32).abs() < tolerance as i32;
        let on_top = (py as i32 - self.y as i32).abs() < tolerance as i32;
        let on_bottom = (py as i32 - (self.y + self.h) as i32).abs() < tolerance as i32;

        let within_horizontal = px >= self.x && px <= self.x + self.w;
        let within_vertical = py >= self.y && py <= self.y + self.h;

        if on_top && on_left {
            Some(ResizeHandle::TopLeft)
        } else if on_top && on_right {
            Some(ResizeHandle::TopRight)
        } else if on_bottom && on_left {
            Some(ResizeHandle::BottomLeft)
        } else if on_bottom && on_right {
            Some(ResizeHandle::BottomRight)
        } else if on_top && within_horizontal {
            Some(ResizeHandle::Top)
        } else if on_bottom && within_horizontal {
            Some(ResizeHandle::Bottom)
        } else if on_left && within_vertical {
            Some(ResizeHandle::Left)
        } else if on_right && within_vertical {
            Some(ResizeHandle::Right)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ResizeHandle {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DragMode {
    None,
    Selecting,
    MovingSelection,
    ResizingSelection(ResizeHandle),
}

impl Default for DragMode {
    fn default() -> Self {
        DragMode::None
    }
}

#[derive(Default)]
struct AppState {
    last_window_size: LogicalSize,
    last_window_position: LogicalPosition,
    last_thumbnail_window_position: LogicalPosition,
    last_thumbnail_window_size: LogicalSize,
    image_list: Vec<PathBuf>,
    current_image_index: Option<usize>,
    selection: Option<SelectionRect>,
    drag_mode: DragMode,
    selection_start_point: Option<(u32, u32)>,
    initial_selection_on_drag: Option<SelectionRect>,
    initial_mouse_on_drag: Option<(u32, u32)>,
    thumbnail_receiver: Option<mpsc::Receiver<ThumbnailMessage>>,
    is_from_clipboard: bool,
    tick_count: u64,
}

enum ThumbnailMessage {
    Data(SharedPixelBuffer<Rgba8Pixel>, String),
    Completion,
}

fn load_image_to_slint(img: DynamicImage) -> Image {
    let img_data = img.to_rgba8();
    Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
        img_data.as_raw(),
        img_data.width(),
        img_data.height(),
    ))
}

fn buffer_to_slint_image(buffer: SharedPixelBuffer<Rgba8Pixel>) -> Image {
    Image::from_rgba8(buffer)
}

fn update_image_info(ui: &AppWindow) {
    if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
        ui.invoke_update_image_info();
        let width = ui.get_image_w();
        let height = ui.get_image_h();
        let scale = ui.get_image_scale();
        ui.set_info_text(
            format!(
                "{:>0.1}% | {} x {} | {} x {}",
                scale * 100.0,
                width,
                height,
                pixel_buffer.width(),
                pixel_buffer.height()
            )
            .into(),
        );
    }
}

fn save_layout(
    ui: &AppWindow,
    thumb_ui: &ThumbnailWindow,
    volatile_settings: &Rc<RefCell<VolatileSettings>>,
    slot: usize,
) {
    let scale_factor = ui.window().scale_factor();
    let thumb_scale_factor = thumb_ui.window().scale_factor();

    let layout = voutil::settings::WindowLayout {
        window_position: slint::LogicalPosition::from_physical(ui.window().position(), scale_factor),
        window_size: slint::LogicalSize::from_physical(ui.window().size(), scale_factor),
        thumbnail_window_position: slint::LogicalPosition::from_physical(
            thumb_ui.window().position(),
            thumb_scale_factor,
        ),
        thumbnail_window_size: slint::LogicalSize::from_physical(
            thumb_ui.window().size(),
            thumb_scale_factor,
        ),
        thumbnail_window_visible: thumb_ui.window().is_visible(),
    };

    let mut volatile = volatile_settings.borrow_mut();
    if slot < volatile.layouts.len() {
        log::info!("Saving layout to slot {}", slot + 1);
        volatile.layouts[slot] = Some(layout);
        match volatile.save_blocking() {
            Ok(_) => {
                log::info!("Successfully saved volatile settings to disk.");
                ui.set_status_text(format!("Layout {} saved.", slot + 1).into());
            }
            Err(e) => {
                log::error!("Failed to save volatile settings: {}", e);
                ui.set_status_text(format!("Failed to save Layout {}: {}", slot + 1, e).into());
            }
        }
    } else {
        log::warn!("Layout slot {} out of bounds", slot + 1);
    }
}

fn load_layout(
    ui: &AppWindow,
    thumb_ui: &ThumbnailWindow,
    volatile_settings: &Rc<RefCell<VolatileSettings>>,
    slot: usize,
) {
    let volatile = volatile_settings.borrow();
    if slot < volatile.layouts.len() {
        if let Some(layout) = &volatile.layouts[slot] {
            ui.window().set_position(layout.window_position);
            ui.window().set_size(layout.window_size);
            thumb_ui
                .window()
                .set_position(layout.thumbnail_window_position);
            thumb_ui.window().set_size(layout.thumbnail_window_size);
            if layout.thumbnail_window_visible {
                let _ = thumb_ui.show();
            } else {
                let _ = thumb_ui.hide();
            }
            ui.set_status_text(format!("Layout {} loaded.", slot + 1).into());
        } else {
            ui.set_status_text(format!("Layout {} is empty.", slot + 1).into());
        }
    }
}

fn apply_ui_theme(
    main_ui: &AppWindow,
    settings_ui: &SettingsWindow,
    thumb_ui: &ThumbnailWindow,
    color_correction_ui: &ColorCorrectionWindow,
    choice: voutil::settings::ColorTheme,
    accent_color: [u8; 3],
) {
    let is_dark = match choice {
        voutil::settings::ColorTheme::Dark => true,
        voutil::settings::ColorTheme::Light => false,
        voutil::settings::ColorTheme::System => match dark_light::detect() {
            Ok(dark_light::Mode::Dark) => true,
            Ok(dark_light::Mode::Light) => false,
            _ => true, // Fallback to dark
        },
    };

    let slint_accent_color = slint::Color::from_rgb_u8(accent_color[0], accent_color[1], accent_color[2]);
    main_ui.global::<Theme>().set_accent_color(slint_accent_color.clone());
    settings_ui.global::<Theme>().set_accent_color(slint_accent_color.clone());
    thumb_ui.global::<Theme>().set_accent_color(slint_accent_color.clone());
    color_correction_ui.global::<Theme>().set_accent_color(slint_accent_color);

    // Only update if the theme state has actually changed
    if main_ui.global::<Theme>().get_is_dark() != is_dark {
        main_ui.global::<Theme>().set_is_dark(is_dark);
        settings_ui.global::<Theme>().set_is_dark(is_dark);
        thumb_ui.global::<Theme>().set_is_dark(is_dark);
        color_correction_ui.global::<Theme>().set_is_dark(is_dark);

        // Update standard Slint Palette for widgets like ComboBox
        let scheme = if is_dark {
            slint::language::ColorScheme::Dark
        } else {
            slint::language::ColorScheme::Light
        };
        
        main_ui.global::<Palette>().set_color_scheme(scheme);
        settings_ui.global::<Palette>().set_color_scheme(scheme);
        thumb_ui.global::<Palette>().set_color_scheme(scheme);
        color_correction_ui.global::<Palette>().set_color_scheme(scheme);
    }
}

fn set_image(
    ui: &AppWindow,
    thumbnail_window: &ThumbnailWindow,
    app_state: &mut AppState,
    path: PathBuf,
    volatile_settings: &Rc<RefCell<VolatileSettings>>,
    persistent_settings: &Rc<RefCell<PersistentSettings>>,
) -> bool {
    app_state.selection = None;

    // Prevent opening thumbnail cache files directly
    if let Ok(cache_dir) = cache::get_cache_dir() {
        if let Some(parent) = path.parent() {
            if parent == &cache_dir {
                ui.set_status_text("Cannot open thumbnail cache files.".into());
                return false;
            }
        }
    }

    if let Ok(img) = {
        let extension = path.extension().unwrap_or_default().to_str().unwrap_or_default().to_lowercase();
        if extension == "heif" || extension == "heic" {
            #[cfg(feature = "heif")]
            {
                use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
                let lib_heif = LibHeif::new();
                let ctx = HeifContext::read_from_file(&path.to_string_lossy());
                let result: Result<DynamicImage, anyhow::Error> = ctx.map_err(|e| anyhow::anyhow!("{}", e)).and_then(|ctx| {
                    let handle = ctx.primary_image_handle().map_err(|e| anyhow::anyhow!("{}", e))?;
                    let img = lib_heif.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None).map_err(|e| anyhow::anyhow!("{}", e))?;
                    let planes = img.planes();
                    let interleaved = planes.interleaved.ok_or_else(|| anyhow::anyhow!("Can't create interleaved plane"))?;
                    let data = interleaved.data;
                    let width = interleaved.width;
                    let height = interleaved.height;
                    let stride = interleaved.stride;

                    let mut res: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
                    for y in 0..height {
                        let mut step = y as usize * stride;
                        for _ in 0..width {
                            res.extend_from_slice(&[
                                data[step],
                                data[step + 1],
                                data[step + 2],
                                data[step + 3],
                            ]);
                            step += 4;
                        }
                    }
                    let buf = image::ImageBuffer::from_vec(width, height, res)
                        .ok_or_else(|| anyhow::anyhow!("Can't create HEIC/HEIF ImageBuffer"))?;
                    Ok(DynamicImage::ImageRgba8(buf))
                });
                
                result
            }
            #[cfg(not(feature = "heif"))]
            {
                image::open(&path).map_err(|e| anyhow::anyhow!("{}", e))
            }
        } else {
            image::open(&path).map_err(|e| anyhow::anyhow!("{}", e))
        }
    } {
        let new_slint_image = load_image_to_slint(img);
        volatile_settings.borrow_mut().last_image_path = path.clone();
        let _ = volatile_settings.borrow_mut().save_blocking();
        let parent_dir = path.parent().unwrap_or(&path).to_path_buf();
        let thumb_ui_handle = thumbnail_window.as_weak();

        // Update current directory and save to volatile settings
        if let Ok(current_dir) = std::env::current_dir() {
            if current_dir != parent_dir {
                let _ = std::env::set_current_dir(&parent_dir);
                volatile_settings.borrow_mut().last_open_directory = parent_dir.clone();
                let _ = volatile_settings.borrow_mut().save_blocking();
            }
        } else {
            // Handle error case for current_dir, e.g., set to parent_dir and save
            let _ = std::env::set_current_dir(&parent_dir);
            volatile_settings.borrow_mut().last_open_directory = parent_dir.clone();
            let _ = volatile_settings.borrow_mut().save_blocking();
        }

        // Always re-scan and update thumbnail list if directory changes or is empty
        if app_state.image_list.is_empty()
            || app_state.image_list[0]
                .parent()
                .map_or(true, |p| p != parent_dir)
        {
            let image_list = if let Ok(entries) = std::fs::read_dir(&parent_dir) {
                let supported_extensions: std::collections::HashSet<&str> = [
                    "pnm", "pgm", "ppm", "pam", "png", "jpg", "jpeg", "gif", "webp", "tif",
                    "tiff", "tga", "dds", "bmp", "ico", "hdr", "exr", "ff", "farbfeld", "avif", "qoi",
                    "heif", "heic"
                ].iter().cloned().collect();

                let mut paths: Vec<_> = entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension().map_or(false, |ext| {
                                supported_extensions.contains(ext.to_str().unwrap_or("").to_lowercase().as_str())
                            })
                    })
                    .collect();

                let settings = persistent_settings.borrow();
                if !settings.use_os_sorting {
                    match settings.sort_criteria.as_str() {
                        "Modified time" => {
                            paths.sort_by_key(|p| {
                                std::fs::metadata(p)
                                    .and_then(|m| m.modified())
                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                            });
                        }
                        "Created time" => {
                            paths.sort_by_key(|p| {
                                std::fs::metadata(p)
                                    .and_then(|m| m.created())
                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                            });
                        }
                        "Size" => {
                            paths.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
                        }
                        _ => {
                            // Default to Name
                            paths.sort();
                        }
                    }

                    if settings.sort_order == "Descending" {
                        paths.reverse();
                    }
                } else {
                    // Use default (alphabetical ascending)
                    paths.sort();
                }
                paths
            } else {
                vec![]
            };

            let image_list_clone = image_list.clone();
            if let Some(thumb_ui) = thumb_ui_handle.upgrade() {
                thumb_ui.set_thumbnails(Rc::new(VecModel::default()).into());
                let (tx, rx) = mpsc::channel();
                app_state.thumbnail_receiver = Some(rx); // Store receiver in AppState

                thread::spawn(move || {
                    let cache_dir = match cache::get_cache_dir() {
                        Ok(dir) => Some(dir),
                        Err(e) => {
                            eprintln!("[Voutil] Failed to get/create cache directory: {}. Thumbnails will not be cached.", e);
                            None
                        }
                    };

                    image_list_clone.par_iter().for_each(|p| {
                        let mut loaded_from_cache = false;
                        let mut final_buffer: Option<SharedPixelBuffer<Rgba8Pixel>> = None;
                        let cache_dir_clone = cache_dir.clone();

                        // Try to load from cache
                        if let Some(ref dir) = cache_dir_clone {
                            if let Some(thumb_path) = cache::get_thumbnail_path(p, dir) {
                                if thumb_path.exists() {
                                    if let Ok(img) = image::open(thumb_path) {
                                        let rgba_image = img.to_rgba8();
                                        final_buffer = Some(SharedPixelBuffer::clone_from_slice(
                                            rgba_image.as_raw(),
                                            rgba_image.width(),
                                            rgba_image.height(),
                                        ));
                                        loaded_from_cache = true;
                                    }
                                }
                            }
                        }

                        // If not loaded, generate new and try to save to cache
                        if !loaded_from_cache {
                            if let Ok(img) = {
                                let extension = p.extension().unwrap_or_default().to_str().unwrap_or_default().to_lowercase();
                                if extension == "heif" || extension == "heic" {
                                    #[cfg(feature = "heif")]
                                    {
                                        use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
                                        let lib_heif = LibHeif::new();
                                        let ctx = HeifContext::read_from_file(&p.to_string_lossy());
                                        let result: Result<DynamicImage, anyhow::Error> = ctx.map_err(|e| anyhow::anyhow!("{}", e)).and_then(|ctx| {
                                            let handle = ctx.primary_image_handle().map_err(|e| anyhow::anyhow!("{}", e))?;
                                            let img = lib_heif.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None).map_err(|e| anyhow::anyhow!("{}", e))?;
                                            let planes = img.planes();
                                            let interleaved = planes.interleaved.ok_or_else(|| anyhow::anyhow!("Can't create interleaved plane"))?;
                                            let data = interleaved.data;
                                            let width = interleaved.width;
                                            let height = interleaved.height;
                                            let stride = interleaved.stride;

                                            let mut res: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
                                            for y in 0..height {
                                                let mut step = y as usize * stride;
                                                for _ in 0..width {
                                                    res.extend_from_slice(&[
                                                        data[step],
                                                        data[step + 1],
                                                        data[step + 2],
                                                        data[step + 3],
                                                    ]);
                                                    step += 4;
                                                }
                                            }
                                            let buf = image::ImageBuffer::from_vec(width, height, res)
                                                .ok_or_else(|| anyhow::anyhow!("Can't create HEIC/HEIF ImageBuffer"))?;
                                            Ok(DynamicImage::ImageRgba8(buf))
                                        });
                                        
                                        result
                                    }
                                    #[cfg(not(feature = "heif"))]
                                    {
                                        image::open(p).map_err(|e| anyhow::anyhow!("{}", e))
                                    }
                                } else {
                                    image::open(p).map_err(|e| anyhow::anyhow!("{}", e))
                                }
                            } {
                                let thumb = img.thumbnail(120, 120);
                                let rgba_image = thumb.to_rgba8();

                                if let Some(ref dir) = cache_dir_clone {
                                    if let Some(thumb_path) = cache::get_thumbnail_path(p, dir) {
                                        // Save as WebP for efficiency and transparency support
                                        if let Err(e) = rgba_image
                                            .save_with_format(&thumb_path, image::ImageFormat::WebP)
                                        {
                                            eprintln!(
                                                "[Voutil] Failed to save thumbnail for {:?}: {}",
                                                p, e
                                            );
                                        }
                                    }
                                }

                                final_buffer = Some(SharedPixelBuffer::clone_from_slice(
                                    rgba_image.as_raw(),
                                    rgba_image.width(),
                                    rgba_image.height(),
                                ));
                            }
                        }

                        if let Some(buffer) = final_buffer {
                            if let Err(e) = tx.send(ThumbnailMessage::Data(
                                buffer,
                                p.to_string_lossy().to_string(),
                            )) {
                                eprintln!(
                                    "[Voutil] Failed to send thumbnail (data) to UI thread: {}",
                                    e
                                );
                            }
                        }
                    });
                    // Send completion signal after all thumbnails are processed
                    let _ = tx.send(ThumbnailMessage::Completion);
                });
            }
            app_state.image_list = image_list;
        }

        app_state.current_image_index = app_state.image_list.iter().position(|p| p == &path);
        app_state.is_from_clipboard = false;
        ui.set_auto_fit(true);
        ui.set_image_display(new_slint_image);
        
        let title = if let Some(filename) = path.file_name() {
            format!("{} - Voutil", filename.to_string_lossy())
        } else {
            "Voutil".to_string()
        };
        ui.set_window_title(title.into());

        update_image_info(&ui);
        ui.set_status_text(format!("Loaded: {}", path.to_string_lossy()).into());
        thumbnail_window.set_selected_path(path.to_string_lossy().as_ref().into());
        if let Some(index) = app_state.current_image_index {
            thumbnail_window.set_selected_index(index as i32);
        }
        true
    } else {
        app_state.current_image_index = app_state.image_list.iter().position(|p| p == &path);
        
        let title = if let Some(filename) = path.file_name() {
            format!("{} - Voutil", filename.to_string_lossy())
        } else {
            "Voutil".to_string()
        };
        ui.set_window_title(title.into());

        ui.set_status_text(format!("Failed to load: {}", path.to_string_lossy()).into());
        thumbnail_window.set_selected_path(path.to_string_lossy().as_ref().into());
        if let Some(index) = app_state.current_image_index {
            thumbnail_window.set_selected_index(index as i32);
        }
        false
    }
}

fn show_next_image(
    ui: &AppWindow,
    thumb_ui: &ThumbnailWindow,
    app_state: &mut AppState,
    volatile_settings: &Rc<RefCell<VolatileSettings>>,
    persistent_settings: &Rc<RefCell<PersistentSettings>>,
) {
    if let Some(index) = app_state.current_image_index {
        if !app_state.image_list.is_empty() {
            let mut next_index = (index + 1) % app_state.image_list.len();
            while next_index != index {
                let path = app_state.image_list[next_index].clone();
                if set_image(
                    ui,
                    thumb_ui,
                    app_state,
                    path,
                    volatile_settings,
                    persistent_settings,
                ) {
                    break;
                }
                next_index = (next_index + 1) % app_state.image_list.len();
            }
        }
    }
}

fn show_previous_image(
    ui: &AppWindow,
    thumb_ui: &ThumbnailWindow,
    app_state: &mut AppState,
    volatile_settings: &Rc<RefCell<VolatileSettings>>,
    persistent_settings: &Rc<RefCell<PersistentSettings>>,
) {
    if let Some(index) = app_state.current_image_index {
        if !app_state.image_list.is_empty() {
            let mut prev_index = (index + app_state.image_list.len() - 1) % app_state.image_list.len();
            while prev_index != index {
                let path = app_state.image_list[prev_index].clone();
                if set_image(
                    ui,
                    thumb_ui,
                    app_state,
                    path,
                    volatile_settings,
                    persistent_settings,
                ) {
                    break;
                }
                prev_index = (prev_index + app_state.image_list.len() - 1) % app_state.image_list.len();
            }
        }
    }
}

fn populate_shortcut_list(settings_window: &SettingsWindow, shortcuts: &voutil::shortcuts::Shortcuts) {
    let mut entries = Vec::new();
    for (event, key_list) in shortcuts {
        let keys: Vec<slint::SharedString> = key_list.iter().map(|k| k.format().into()).collect();
        entries.push(ShortcutEntry {
            description: event.description().into(),
            event_type: format!("{:?}", event).into(),
            keys: Rc::new(slint::VecModel::from(keys)).into(),
        });
    }
    settings_window.set_shortcut_list(Rc::new(slint::VecModel::from(entries)).into());
}

fn parse_serial_number(stem: &str) -> (String, Option<u32>) {
    if let Some(open_paren) = stem.rfind(" (") {
        if stem.ends_with(')') {
            let num_str = &stem[open_paren + 2..stem.len() - 1];
            if let Ok(n) = num_str.parse::<u32>() {
                return (stem[..open_paren].to_string(), Some(n));
            }
        }
    }
    (stem.to_string(), None)
}

fn main() -> Result<(), slint::PlatformError> {
    let persistent_settings = Rc::new(RefCell::new(PersistentSettings::load().unwrap_or_default()));
    let volatile_settings = Rc::new(RefCell::new(VolatileSettings::load().unwrap_or_default()));
    let main_window = AppWindow::new()?;
    let settings_window = SettingsWindow::new()?;
    let thumbnail_window = ThumbnailWindow::new()?;
    let color_correction_window = ColorCorrectionWindow::new()?;
    let app_state = Rc::new(RefCell::new(AppState::default()));

    // --- Initial state setup from settings ---
    let mut initial_pos = volatile_settings.borrow().window_position;
    let initial_size = volatile_settings.borrow().window_size;
    if initial_pos.x < 0.0 || initial_pos.y < 0.0 {
        initial_pos = VolatileSettings::default().window_position;
    }
    main_window.window().set_position(initial_pos);
    main_window.window().set_size(initial_size);

    let mut thumb_initial_pos = volatile_settings.borrow().thumbnail_window_position;
    let thumb_initial_size = volatile_settings.borrow().thumbnail_window_size;
    if thumb_initial_pos.x < 0.0 || thumb_initial_pos.y < 0.0 {
        thumb_initial_pos = VolatileSettings::default().thumbnail_window_position;
    }
    thumbnail_window.window().set_position(thumb_initial_pos);
    thumbnail_window.window().set_size(thumb_initial_size);
    if volatile_settings.borrow().thumbnail_window_visible {
        let _ = thumbnail_window.show();
    }

    // --- Initialize AppState with current window geometry ---
    app_state.borrow_mut().last_window_position = initial_pos;
    app_state.borrow_mut().last_window_size = initial_size;
    app_state.borrow_mut().last_thumbnail_window_position = thumb_initial_pos;
    app_state.borrow_mut().last_thumbnail_window_size = thumb_initial_size;

    // --- Initial state setup ---
    main_window.set_status_text("Ready. Open a file to begin.".into());
    
    let current_theme = persistent_settings.borrow().theme.clone();
    let current_accent = persistent_settings.borrow().accent_color;
    settings_window.set_selected_theme(match current_theme {
        voutil::settings::ColorTheme::Light => "Light".into(),
        voutil::settings::ColorTheme::System => "System".into(),
        _ => "Dark".into(),
    });
    apply_ui_theme(&main_window, &settings_window, &thumbnail_window, &color_correction_window, current_theme, current_accent);

    main_window.set_show_status_messages(persistent_settings.borrow().show_status_messages);

    settings_window.set_accent_r(current_accent[0] as i32);
    settings_window.set_accent_g(current_accent[1] as i32);
    settings_window.set_accent_b(current_accent[2] as i32);
    settings_window.set_accent_hex(format!("#{:02X}{:02X}{:02X}", current_accent[0], current_accent[1], current_accent[2]).into());
    settings_window.set_show_status_messages(persistent_settings.borrow().show_status_messages);
    settings_window.set_reopen_last_image(persistent_settings.borrow().reopen_last_image);
    settings_window.set_use_os_sorting(persistent_settings.borrow().use_os_sorting);
    settings_window.set_sort_criteria(persistent_settings.borrow().sort_criteria.clone().into());
    settings_window.set_sort_order(persistent_settings.borrow().sort_order.clone().into());
    settings_window.set_crop_aspect_ratio(persistent_settings.borrow().crop_aspect_ratio.clone().into());
    settings_window.set_default_save_format(persistent_settings.borrow().default_save_format.clone().into());
    settings_window.set_jpeg_quality(persistent_settings.borrow().jpeg_quality as i32);
    settings_window.set_selected_zoom_mode(match persistent_settings.borrow().zoom_mode {
        voutil::settings::ZoomMode::Multiplier => "Multiplier".into(),
        voutil::settings::ZoomMode::Additive => "Additive".into(),
    });
    settings_window.set_zoom_value(persistent_settings.borrow().zoom_value);
    populate_shortcut_list(&settings_window, &persistent_settings.borrow().shortcuts);

    // Set initial current directory based on last opened directory
    let _ = std::env::set_current_dir(volatile_settings.borrow().last_open_directory.clone());

    // --- Handle command line arguments ---
    if let Some(path_str) = env::args().nth(1) {
        set_image(
            &main_window,
            &thumbnail_window,
            &mut app_state.borrow_mut(),
            PathBuf::from(path_str),
            &volatile_settings.clone(),
            &persistent_settings.clone(),
        );
    }

    // --- Main window callbacks ---
    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone = persistent_settings.clone();

    let shortcut_settings_clone = persistent_settings.clone();
    let settings_window_handle_for_record = settings_window.as_weak();
    settings_window.on_key_recorded(move |text, ctrl, alt, shift| {
        if let Some(sw) = settings_window_handle_for_record.upgrade() {
            let event_type_str = sw.get_recording_event_type();
            if event_type_str != "" {
                // If only a modifier is pressed, don't finish yet
                if voutil::shortcuts::is_modifier(&text) {
                    return true;
                }

                let mut settings = shortcut_settings_clone.borrow_mut();
                let mut target_event = None;
                for event in settings.shortcuts.keys() {
                    if format!("{:?}", event) == event_type_str.as_str() {
                        target_event = Some(event.clone());
                        break;
                    }
                }

                if let Some(event) = target_event {
                    let new_keypress = voutil::shortcuts::SimultaneousKeypresses {
                        key: voutil::shortcuts::slint_to_human_readable(&text),
                        ctrl,
                        alt,
                        shift,
                    };
                    
                    let key_list = settings.shortcuts.entry(event).or_insert_with(|| Vec::new());
                    // Add only if not already exists
                    if !key_list.contains(&new_keypress) {
                        key_list.push(new_keypress);
                        let _ = settings.save_blocking();
                        populate_shortcut_list(&sw, &settings.shortcuts);
                    }
                }

                sw.set_recording_event_type("".into());
                sw.set_recording_description("".into());
                return true;
            }
        }
        false
    });
    let volatile_settings_clone_for_thumb_shortcuts = volatile_settings.clone();
    let thumb_ui_handle_for_shortcuts = thumbnail_window.as_weak();
    let main_window_handle_for_thumb_shortcuts = main_window.as_weak();
    let persistent_settings_clone_for_thumb_shortcuts = persistent_settings.clone();
    thumbnail_window.on_shortcut_pressed(move |text, ctrl, alt, shift| {
        if let Some(ui) = main_window_handle_for_thumb_shortcuts.upgrade() {
            if let Some(command) = lookup(
                &persistent_settings_clone_for_thumb_shortcuts.borrow().shortcuts,
                &text,
                ctrl,
                alt,
                shift,
            ) {
                match command {
                    InputEvent::SaveLayout1 => {
                        if let Some(tw) = thumb_ui_handle_for_shortcuts.upgrade() {
                            save_layout(&ui, &tw, &volatile_settings_clone_for_thumb_shortcuts, 0);
                        }
                        return true;
                    }
                    InputEvent::SaveLayout2 => {
                        if let Some(tw) = thumb_ui_handle_for_shortcuts.upgrade() {
                            save_layout(&ui, &tw, &volatile_settings_clone_for_thumb_shortcuts, 1);
                        }
                        return true;
                    }
                    InputEvent::SaveLayout3 => {
                        if let Some(tw) = thumb_ui_handle_for_shortcuts.upgrade() {
                            save_layout(&ui, &tw, &volatile_settings_clone_for_thumb_shortcuts, 2);
                        }
                        return true;
                    }
                    InputEvent::LoadLayout1 => {
                        if let Some(tw) = thumb_ui_handle_for_shortcuts.upgrade() {
                            load_layout(&ui, &tw, &volatile_settings_clone_for_thumb_shortcuts, 0);
                        }
                        return true;
                    }
                    InputEvent::LoadLayout2 => {
                        if let Some(tw) = thumb_ui_handle_for_shortcuts.upgrade() {
                            load_layout(&ui, &tw, &volatile_settings_clone_for_thumb_shortcuts, 1);
                        }
                        return true;
                    }
                    InputEvent::LoadLayout3 => {
                        if let Some(tw) = thumb_ui_handle_for_shortcuts.upgrade() {
                            load_layout(&ui, &tw, &volatile_settings_clone_for_thumb_shortcuts, 2);
                        }
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    });

    let shortcut_settings_clone = persistent_settings.clone();
    let shortcut_ui_handle = main_window.as_weak();
    let shortcut_settings_window_handle = settings_window.as_weak();
    let shortcut_thumbnail_window_handle = thumbnail_window.as_weak();
    let volatile_settings_clone_for_shortcuts = volatile_settings.clone();
    main_window.on_shortcut_pressed(move |text, ctrl, alt, shift| {
        log::info!("Shortcut callback: text='{:?}', ctrl={}, alt={}, shift={}", text, ctrl, alt, shift);
        if let Some(ui) = shortcut_ui_handle.upgrade() {
            // Check if we are recording a shortcut in settings window
            if let Some(sw) = shortcut_settings_window_handle.upgrade() {
                let event_type_str = sw.get_recording_event_type();
                if event_type_str != "" {
                    // If only a modifier is pressed, don't finish yet
                    if voutil::shortcuts::is_modifier(&text) {
                        return true;
                    }

                    let mut settings = shortcut_settings_clone.borrow_mut();
                    let mut target_event = None;
                    for event in settings.shortcuts.keys() {
                        if format!("{:?}", event) == event_type_str.as_str() {
                            target_event = Some(event.clone());
                            break;
                        }
                    }

                    if let Some(event) = target_event {
                        let new_keypress = voutil::shortcuts::SimultaneousKeypresses {
                            key: voutil::shortcuts::slint_to_human_readable(&text),
                            ctrl,
                            alt,
                            shift,
                        };
                        
                        let key_list = settings.shortcuts.entry(event).or_insert_with(|| Vec::new());
                        // Add only if not already exists
                        if !key_list.contains(&new_keypress) {
                            key_list.push(new_keypress);
                            let _ = settings.save_blocking();
                            populate_shortcut_list(&sw, &settings.shortcuts);
                        }
                    }

                    sw.set_recording_event_type("".into());
                    sw.set_recording_description("".into());
                    return true;
                }
            }

            if let Some(command) = lookup(&shortcut_settings_clone.borrow().shortcuts, &text, ctrl, alt, shift) {
                match command {
                    InputEvent::OpenFile => ui.invoke_request_open_file(),
                    InputEvent::Fullscreen => {
                        let current = ui.get_fullscreen_enabled();
                        ui.set_fullscreen_enabled(!current);
                    }
                    InputEvent::AlwaysOnTop => {
                        let current = ui.get_always_on_top_enabled();
                        ui.set_always_on_top_enabled(!current);
                    }
                    InputEvent::ToggleThumbnails => ui.invoke_show_thumbnail_window(),
                    InputEvent::SaveLayout1 => {
                        if let Some(tw) = shortcut_thumbnail_window_handle.upgrade() {
                            save_layout(&ui, &tw, &volatile_settings_clone_for_shortcuts, 0);
                        }
                    }
                    InputEvent::SaveLayout2 => {
                        if let Some(tw) = shortcut_thumbnail_window_handle.upgrade() {
                            save_layout(&ui, &tw, &volatile_settings_clone_for_shortcuts, 1);
                        }
                    }
                    InputEvent::SaveLayout3 => {
                        if let Some(tw) = shortcut_thumbnail_window_handle.upgrade() {
                            save_layout(&ui, &tw, &volatile_settings_clone_for_shortcuts, 2);
                        }
                    }
                    InputEvent::LoadLayout1 => {
                        if let Some(tw) = shortcut_thumbnail_window_handle.upgrade() {
                            load_layout(&ui, &tw, &volatile_settings_clone_for_shortcuts, 0);
                        }
                    }
                    InputEvent::LoadLayout2 => {
                        if let Some(tw) = shortcut_thumbnail_window_handle.upgrade() {
                            load_layout(&ui, &tw, &volatile_settings_clone_for_shortcuts, 1);
                        }
                    }
                    InputEvent::LoadLayout3 => {
                        if let Some(tw) = shortcut_thumbnail_window_handle.upgrade() {
                            load_layout(&ui, &tw, &volatile_settings_clone_for_shortcuts, 2);
                        }
                    }
                    InputEvent::CropSelection => ui.invoke_crop_in_place(),
                    InputEvent::Exit => {
                        if ui.get_fullscreen_enabled() {
                            ui.invoke_reset_view();
                            let before = ui.get_zen_mode_before_enter();
                            ui.set_zen_mode_enabled(before);
                            ui.set_fullscreen_enabled(false);
                        } else if ui.get_thumbnail_window_is_visible() {
                            ui.invoke_show_thumbnail_window();
                        } else {
                            ui.invoke_exit();
                        }
                    }
                    InputEvent::PreviousImage => ui.invoke_previous_image(),
                    InputEvent::NextImage => ui.invoke_next_image(),
                    InputEvent::ZoomActualSize => {
                        ui.invoke_apply_view_one_to_one_properties();
                        ui.invoke_view_one_to_one();
                    }
                    InputEvent::ResetView => ui.invoke_reset_view(),
                    InputEvent::Copy => ui.invoke_copy_to_clipboard(),
                    InputEvent::Paste => ui.invoke_paste_from_clipboard(),
                    InputEvent::SaveAs => ui.invoke_save_as(),
                    InputEvent::Resize => ui.invoke_show_resize(),
                    InputEvent::ColorCorrections => ui.invoke_show_color_correction_window(),
                    InputEvent::Preferences => ui.invoke_show_settings_window(),
                    InputEvent::BrowseToFileLocation => ui.invoke_browse_to_file_location(),
                    InputEvent::ZenMode => {
                        let current = ui.get_zen_mode_enabled();
                        ui.set_zen_mode_enabled(!current);
                    }
                    InputEvent::PerfectFullscreen => {
                        ui.invoke_reset_view();
                        if ui.get_fullscreen_enabled() {
                            let before = ui.get_zen_mode_before_enter();
                            ui.set_zen_mode_enabled(before);
                            ui.set_fullscreen_enabled(false);
                        } else {
                            ui.set_zen_mode_before_enter(ui.get_zen_mode_enabled());
                            ui.set_zen_mode_enabled(true);
                            ui.set_fullscreen_enabled(true);
                        }
                    }
                }
                return true;
            }
        }
        false
    });

    main_window.on_request_open_file(move || {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(std::env::current_dir().unwrap_or_default())
                .pick_file()
            {
                set_image(
                    &ui,
                    &thumb_ui,
                    &mut app_state_clone.borrow_mut(),
                    path,
                    &volatile_settings_clone,
                    &persistent_settings_clone,
                );
            } else {
                ui.set_status_text("File open cancelled.".into());
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let persistent_settings_clone = persistent_settings.clone();
    let volatile_settings_clone = volatile_settings.clone();
    main_window.on_save_as(move || {
        if let (Some(ui), Some(thumb_ui)) = (main_window_handle.upgrade(), thumbnail_window_handle.upgrade()) {
            if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
                let default_format = persistent_settings_clone.borrow().default_save_format.clone();
                
                let mut dialog = rfd::FileDialog::new();
                let filters = [
                    ("PNG Image", vec!["png"]),
                    ("JPEG Image", vec!["jpg", "jpeg"]),
                    ("BMP Image", vec!["bmp"]),
                    ("WebP Image", vec!["webp"]),
                ];

                let preferred_name = match default_format.as_str() {
                    "Jpg" => "JPEG Image",
                    "Bmp" => "BMP Image",
                    "WebP" => "WebP Image",
                    _ => "PNG Image",
                };

                // Add preferred filter first to make it the default in the dialog
                for (name, exts) in &filters {
                    if *name == preferred_name {
                        dialog = dialog.add_filter(*name, exts);
                    }
                }
                for (name, exts) in &filters {
                    if *name != preferred_name {
                        dialog = dialog.add_filter(*name, exts);
                    }
                }

                // Determine base name and target directory in a limited scope to avoid borrow conflict
                let (base_stem, target_dir) = {
                    let app = app_state_clone.borrow();
                    let current_file_path = app.current_image_index.and_then(|idx| app.image_list.get(idx));
                    
                    let base_stem = if app.is_from_clipboard {
                        "Untitled".to_string()
                    } else {
                        current_file_path
                            .and_then(|p| p.file_stem())
                            .and_then(|s| s.to_str())
                            .unwrap_or("Untitled")
                            .to_string()
                    };

                    let target_dir = current_file_path
                        .and_then(|p| p.parent())
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| volatile_settings_clone.borrow().last_open_directory.clone());
                    (base_stem, target_dir)
                };

                let ext = match default_format.as_str() {
                    "Jpg" => "jpg",
                    "Bmp" => "bmp",
                    "WebP" => "webp",
                    _ => "png",
                };

                let (clean_base, existing_num) = parse_serial_number(&base_stem);
                let mut final_name = format!("{}.{}", base_stem, ext);

                // If the file exists, OR if it already had a number (implying it's a copy),
                // we should suggest an incremented number.
                if target_dir.join(&final_name).exists() || existing_num.is_some() {
                    let mut next_num = existing_num.unwrap_or(0) + 1;
                    loop {
                        let candidate = format!("{} ({}).{}", clean_base, next_num, ext);
                        if !target_dir.join(&candidate).exists() {
                            final_name = candidate;
                            break;
                        }
                        next_num += 1;
                    }
                }
                
                if let Some(path) = dialog
                    .set_directory(&target_dir)
                    .set_file_name(final_name)
                    .save_file() 
                {
                    let img_buffer: ImageBuffer<image::Rgba<u8>, _> = ImageBuffer::from_raw(
                        pixel_buffer.width(),
                        pixel_buffer.height(),
                        pixel_buffer.as_bytes().to_vec(),
                    )
                    .unwrap();
                    let dyn_img = DynamicImage::ImageRgba8(img_buffer);

                    let quality = persistent_settings_clone.borrow().jpeg_quality;
                    let encoder = match path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase().as_str() {
                        "jpg" | "jpeg" => FileEncoder::Jpg { quality },
                        "bmp" => FileEncoder::Bmp,
                        "webp" => FileEncoder::WebP,
                        _ => FileEncoder::Png { compressionlevel: CompressionLevel::Default },
                    };

                    if let Err(e) = encoder.save(&dyn_img, &path) {
                        ui.set_status_text(format!("Error saving file: {}", e).into());
                    } else {
                        // After successful save, point the app to the new file
                        // Clear the image list to force set_image to re-scan the directory
                        let mut app = app_state_clone.borrow_mut();
                        app.image_list.clear();
                        set_image(
                            &ui,
                            &thumb_ui,
                            &mut app,
                            path.clone(),
                            &volatile_settings_clone,
                            &persistent_settings_clone,
                        );
                        ui.set_status_text(format!("Saved to {}", path.display()).into());
                    }
                } else {
                    ui.set_status_text("Save cancelled.".into());
                }
            } else {
                ui.set_status_text("No image to save.".into());
            }
        }
    });

    let settings_window_handle_cloned_for_shared_logic = settings_window.as_weak();
    let thumbnail_window_handle_cloned_for_shared_logic = thumbnail_window.as_weak();
    let color_correction_window_handle_cloned_for_shared_logic = color_correction_window.as_weak();
    let close_all_windows_logic = move || {
        if let Some(settings_ui) = settings_window_handle_cloned_for_shared_logic.upgrade() {
            let _ = settings_ui.hide();
        }
        if let Some(thumb_ui) = thumbnail_window_handle_cloned_for_shared_logic.upgrade() {
            let _ = thumb_ui.hide();
        }
        if let Some(cc_ui) = color_correction_window_handle_cloned_for_shared_logic.upgrade() {
            let _ = cc_ui.hide();
        }
        slint::quit_event_loop().unwrap();
    };

    let close_all_windows_logic_clone_for_on_exit = close_all_windows_logic.clone();
    main_window.on_exit(move || {
        close_all_windows_logic_clone_for_on_exit();
    });

    let close_all_windows_logic_clone_for_close_request = close_all_windows_logic.clone();
    main_window.window().on_close_requested(move || {
        close_all_windows_logic_clone_for_close_request();
        slint::CloseRequestResponse::HideWindow // Hide the window and then quit the app
    });

    let main_window_handle = main_window.as_weak();
    main_window.on_reset_view(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            ui.set_auto_fit(true);
            update_image_info(&ui);
            ui.set_status_text("View reset.".into());
        }
    });

    let main_window_handle = main_window.as_weak();
    let volatile_settings_clone = volatile_settings.clone();
    main_window.on_view_one_to_one(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            volatile_settings_clone.borrow_mut().image_scale = 1.0;
            ui.set_auto_fit(false);
            ui.set_image_scale(1.0);
            update_image_info(&ui);
            ui.set_status_text("View 1:1".into());
        }
    });

    let settings_window_handle = settings_window.as_weak();
    let app_state_clone = app_state.clone();
    main_window.on_show_settings_window(move || {
        if let Some(settings_ui) = settings_window_handle.upgrade() {
            let app_state = app_state_clone.borrow();
            
            let main_window_pos = app_state.last_window_position;
            let main_window_size = app_state.last_window_size;

            // Get scale factor to ensure logical pixel consistency
            let scale_factor = settings_ui.window().scale_factor();
            let sw_size = slint::LogicalSize::from_physical(settings_ui.window().size(), scale_factor);

            // Use actual size if already calculated, otherwise fallback to preferred
            let sw_width = if sw_size.width > 0.0 { sw_size.width } else { 720.0 };
            let sw_height = if sw_size.height > 0.0 { sw_size.height } else { 850.0 };

            let mut x = main_window_pos.x + (main_window_size.width - sw_width) / 2.0;
            let mut y = main_window_pos.y + (main_window_size.height - sw_height) / 2.0;

            // Ensure window doesn't go off-screen (especially important for height)
            x = x.max(0.0);
            y = y.max(0.0);

            settings_ui
                .window()
                .set_position(slint::LogicalPosition::new(x, y));
            let _ = settings_ui.show();
            settings_ui.set_focus_trigger(!settings_ui.get_focus_trigger());
        }
    });

    let color_correction_window_handle = color_correction_window.as_weak();
    let app_state_clone = app_state.clone();
    let main_window_handle_for_cc = main_window.as_weak();
    let color_correction_preview_buffer: Rc<RefCell<Option<ImageBuffer<image::Rgba<u8>, Vec<u8>>>>> = Rc::new(RefCell::new(None));
    let cc_preview_buffer_for_show = color_correction_preview_buffer.clone();
    main_window.on_show_color_correction_window(move || {
        if let (Some(cc_ui), Some(main_ui)) = (color_correction_window_handle.upgrade(), main_window_handle_for_cc.upgrade()) {
            cc_ui.invoke_reset();
            if let Some(pixel_buffer) = main_ui.get_image_display().to_rgba8() {
                
                let current_dyn_image = DynamicImage::ImageRgba8(ImageBuffer::from_raw(
                    pixel_buffer.width(),
                    pixel_buffer.height(),
                    pixel_buffer.as_bytes().to_vec(),
                ).unwrap());

                // Use DynamicImage's thumbnail method
                let preview_dyn_img = current_dyn_image.thumbnail(450, 450);
                let preview_buffer = preview_dyn_img.to_rgba8();

                *cc_preview_buffer_for_show.borrow_mut() = Some(preview_buffer.clone());
                
                let slint_img = Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
                    preview_buffer.as_raw(),
                    preview_buffer.width(),
                    preview_buffer.height(),
                ));

                cc_ui.set_original_image(slint_img.clone());
                cc_ui.set_preview_image(slint_img);
                
                let app_state = app_state_clone.borrow();
                let main_window_pos = app_state.last_window_position;
                let main_window_size = app_state.last_window_size;
                
                // ColorCorrectionWindow has preferred-width: 960px and preferred-height: 800px
                let cc_width = 952.0;
                let cc_height = 754.0;

                let x = main_window_pos.x + (main_window_size.width - cc_width) / 2.0;
                let y = main_window_pos.y + (main_window_size.height - cc_height) / 2.0;

                cc_ui.window().set_position(slint::LogicalPosition::new(x, y));
                let _ = cc_ui.show();
                cc_ui.set_focus_trigger(!cc_ui.get_focus_trigger());
            }
        }
    });

    let cc_window_handle = color_correction_window.as_weak();
    let cc_preview_buffer_for_values_changed = color_correction_preview_buffer.clone();
    color_correction_window.on_values_changed(move || {
        if let Some(cc_ui) = cc_window_handle.upgrade() {
            if let Some(original_buffer) = &*cc_preview_buffer_for_values_changed.borrow() { 
                let mut preview_buffer = original_buffer.clone();
                
                // Apply corrections
                apply_color_corrections(
                    &mut preview_buffer,
                    cc_ui.get_brightness(),
                    cc_ui.get_contrast() as f32,
                    cc_ui.get_gamma() as f32,
                    cc_ui.get_red() as f32,
                    cc_ui.get_green() as f32,
                    cc_ui.get_blue() as f32,
                    cc_ui.get_saturation() as f32,
                );

                let slint_img = Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
                    preview_buffer.as_raw(),
                    preview_buffer.width(),
                    preview_buffer.height(),
                ));
                cc_ui.set_preview_image(slint_img);
            }
        }
    });

    let cc_window_handle = color_correction_window.as_weak();
    color_correction_window.on_cancel(move || {
        if let Some(cc_ui) = cc_window_handle.upgrade() {
            let _ = cc_ui.hide();
        }
    });

    let cc_window_handle = color_correction_window.as_weak();
    color_correction_window.on_reset(move || {
        if let Some(cc_ui) = cc_window_handle.upgrade() {
            cc_ui.set_brightness(0.0);
            cc_ui.set_contrast(0.0);
            cc_ui.set_gamma(100.0);
            cc_ui.set_red(0.0);
            cc_ui.set_green(0.0);
            cc_ui.set_blue(0.0);
            cc_ui.set_saturation(0.0);
            cc_ui.invoke_values_changed();
        }
    });

    let main_window_handle = main_window.as_weak();
    let cc_window_handle = color_correction_window.as_weak();
    let app_state_clone = app_state.clone();
    //let cc_preview_buffer_for_apply = color_correction_preview_buffer.clone();
    color_correction_window.on_apply(move || {
        if let (Some(main_ui), Some(cc_ui)) = (main_window_handle.upgrade(), cc_window_handle.upgrade()) {
            if let Some(pixel_buffer) = main_ui.get_image_display().to_rgba8() {
                let mut buffer: ImageBuffer<image::Rgba<u8>, _> = ImageBuffer::from_raw(
                    pixel_buffer.width(),
                    pixel_buffer.height(),
                    pixel_buffer.as_bytes().to_vec(),
                ).unwrap();

                // Apply corrections
                apply_color_corrections(
                    &mut buffer,
                    cc_ui.get_brightness(),
                    cc_ui.get_contrast() as f32,
                    cc_ui.get_gamma() as f32,
                    cc_ui.get_red() as f32,
                    cc_ui.get_green() as f32,
                    cc_ui.get_blue() as f32,
                    cc_ui.get_saturation() as f32,
                );

                let slint_img = Image::from_rgba8(SharedPixelBuffer::clone_from_slice(
                    buffer.as_raw(),
                    buffer.width(),
                    buffer.height(),
                ));
                main_ui.set_image_display(slint_img);
                app_state_clone.borrow_mut().selection = None;
                update_image_info(&main_ui);
                main_ui.set_status_text("Color corrections applied.".into());
                
                let _ = cc_ui.hide();
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    main_window.on_show_resize_window(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
                ui.set_resize_dialog_original_w(pixel_buffer.width() as i32);
                ui.set_resize_dialog_original_h(pixel_buffer.height() as i32);
                ui.set_resize_dialog_new_w(pixel_buffer.width() as i32);
                ui.set_resize_dialog_new_h(pixel_buffer.height() as i32);
                ui.set_resize_dialog_lock_aspect(true);
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    main_window.on_flip_horizontal(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
                let mut app_state = app_state_clone.borrow_mut();
                let mut img_buffer: ImageBuffer<image::Rgba<u8>, _> = ImageBuffer::from_raw(
                    pixel_buffer.width(),
                    pixel_buffer.height(),
                    pixel_buffer.as_bytes().to_vec(),
                )
                .unwrap();

                imageops::flip_horizontal_in_place(&mut img_buffer);
                ui.set_image_display(Image::from_rgba8(
                    SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                        img_buffer.as_raw(),
                        img_buffer.width(),
                        img_buffer.height(),
                    ),
                ));
                app_state.selection = None;
                update_image_info(&ui);
                ui.set_status_text("Image flipped horizontally.".into());
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    main_window.on_rotate_right(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
                let old_w = ui.get_image_w();
                let old_h = ui.get_image_h();
                let old_x = ui.get_image_x();
                let old_y = ui.get_image_y();

                let mut app_state = app_state_clone.borrow_mut();
                let img_buffer: ImageBuffer<image::Rgba<u8>, _> = ImageBuffer::from_raw(
                    pixel_buffer.width(),
                    pixel_buffer.height(),
                    pixel_buffer.as_bytes().to_vec(),
                )
                .unwrap();

                let rotated = imageops::rotate90(&img_buffer);
                ui.set_image_display(Image::from_rgba8(
                    SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                        rotated.as_raw(),
                        rotated.width(),
                        rotated.height(),
                    ),
                ));

                let new_x = old_x + (old_w - old_h) / 2;
                let new_y = old_y + (old_h - old_w) / 2;
                ui.set_image_x(new_x);
                ui.set_image_y(new_y);

                app_state.selection = None;
                update_image_info(&ui);
                ui.set_status_text("Image rotated 90° CW.".into());
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    main_window.on_resize_confirmed(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
                let mut app_state = app_state_clone.borrow_mut();
                let new_w = ui.get_resize_dialog_new_w() as u32;
                let new_h = ui.get_resize_dialog_new_h() as u32;

                let interp = match ui.get_resize_dialog_interpolation().as_str() {
                    "Nearest" => imageops::FilterType::Nearest,
                    "Triangle" => imageops::FilterType::Triangle,
                    "CatmullRom" => imageops::FilterType::CatmullRom,
                    "Gaussian" => imageops::FilterType::Gaussian,
                    _ => imageops::FilterType::Lanczos3,
                };
                let img_buffer: ImageBuffer<image::Rgba<u8>, _> = ImageBuffer::from_raw(
                    pixel_buffer.width(),
                    pixel_buffer.height(),
                    pixel_buffer.as_bytes().to_vec(),
                )
                .unwrap();

                let resized = imageops::resize(&img_buffer, new_w, new_h, interp);
                ui.set_image_display(Image::from_rgba8(
                    SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                        resized.as_raw(),
                        new_w,
                        new_h,
                    ),
                ));
                app_state.selection = None;
                update_image_info(&ui);
                ui.set_status_text("Image resized.".into());
            }
        }
    });

    let volatile_settings_clone = volatile_settings.clone();
    main_window.on_browse_to_file_location(move || {
        let path = volatile_settings_clone.borrow().last_image_path.clone();
        if path.exists() {
            reveal_in_file_manager(&path);
        }
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    main_window.on_copy_to_clipboard(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            let mut app_state = app_state_clone.borrow_mut();
            if let (Some(selection), Some(pixel_buffer)) =
                (app_state.selection, ui.get_image_display().to_rgba8())
            {
                if selection.w > 0 && selection.h > 0 {
                    let img_buffer: ImageBuffer<image::Rgba<u8>, _> = ImageBuffer::from_raw(
                        pixel_buffer.width(),
                        pixel_buffer.height(),
                        pixel_buffer.as_bytes().to_vec(),
                    )
                    .unwrap();
                    let cropped_img = imageops::crop_imm(
                        &img_buffer,
                        selection.x,
                        selection.y,
                        selection.w,
                        selection.h,
                    )
                    .to_image();
                    if let Ok(mut clipboard) = Clipboard::new() {
                        if clipboard
                            .set_image(ImageData {
                                width: cropped_img.width() as usize,
                                height: cropped_img.height() as usize,
                                bytes: Cow::Owned(cropped_img.into_raw()),
                            })
                            .is_ok()
                        {
                            ui.set_status_text("Cropped selection copied.".into());
                        } else {
                            ui.set_status_text("Failed to copy cropped selection.".into());
                        }
                    }
                    app_state.selection = None;
                }
            } else if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
                if let Ok(mut clipboard) = Clipboard::new() {
                    if clipboard
                        .set_image(ImageData {
                            width: pixel_buffer.width() as usize,
                            height: pixel_buffer.height() as usize,
                            bytes: Cow::Owned(pixel_buffer.as_bytes().to_vec()),
                        })
                        .is_ok()
                    {
                        ui.set_status_text("Image copied to clipboard.".into());
                    } else {
                        ui.set_status_text("Failed to copy image.".into());
                    }
                }
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    main_window.on_crop_in_place(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            let mut app_state = app_state_clone.borrow_mut();
            if let (Some(selection), Some(pixel_buffer)) =
                (app_state.selection, ui.get_image_display().to_rgba8())
            {
                if selection.w > 0 && selection.h > 0 {
                    let img_buffer: ImageBuffer<image::Rgba<u8>, _> = ImageBuffer::from_raw(
                        pixel_buffer.width(),
                        pixel_buffer.height(),
                        pixel_buffer.as_bytes().to_vec(),
                    )
                    .unwrap();
                    let cropped_img = imageops::crop_imm(
                        &img_buffer,
                        selection.x,
                        selection.y,
                        selection.w,
                        selection.h,
                    )
                    .to_image();
                    let new_slint_image =
                        load_image_to_slint(DynamicImage::ImageRgba8(cropped_img));
                    volatile_settings_clone.borrow_mut().last_image_path.clear();
                    app_state.selection = None;
                    ui.set_image_display(new_slint_image);
                    ui.set_auto_fit(true);
                    update_image_info(&ui);
                    ui.set_status_text("Image cropped.".into());
                }
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    main_window.on_paste_from_clipboard(move || {
        if let Some(ui) = main_window_handle.upgrade() {
            if let Ok(mut clipboard) = Clipboard::new() {
                if let Ok(clipboard_image) = clipboard.get_image() {
                    let mut app = app_state_clone.borrow_mut();
                    volatile_settings_clone.borrow_mut().last_image_path.clear();
                    app.current_image_index = None;
                    app.is_from_clipboard = true;
                    app.selection = None;
                    ui.set_auto_fit(true);
                    ui.set_window_title("Untitled - Voutil".into());
                    ui.set_image_display(Image::from_rgba8(
                        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                            &clipboard_image.bytes,
                            clipboard_image.width as u32,
                            clipboard_image.height as u32,
                        ),
                    ));
                    update_image_info(&ui);
                    ui.set_status_text("Image pasted from clipboard.".into());
                } else {
                    ui.set_status_text("No image found on clipboard.".into());
                }
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone = persistent_settings.clone();
    main_window.on_next_image(move || {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            let mut app = app_state_clone.borrow_mut();
            show_next_image(&ui, &thumb_ui, &mut app, &volatile_settings_clone, &persistent_settings_clone);
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone = persistent_settings.clone();
    main_window.on_previous_image(move || {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            let mut app = app_state_clone.borrow_mut();
            show_previous_image(&ui, &thumb_ui, &mut app, &volatile_settings_clone, &persistent_settings_clone);
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone = persistent_settings.clone();
    thumbnail_window.on_next_image(move || {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            let mut app = app_state_clone.borrow_mut();
            show_next_image(&ui, &thumb_ui, &mut app, &volatile_settings_clone, &persistent_settings_clone);
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone = persistent_settings.clone();
    thumbnail_window.on_previous_image(move || {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            let mut app = app_state_clone.borrow_mut();
            show_previous_image(&ui, &thumb_ui, &mut app, &volatile_settings_clone, &persistent_settings_clone);
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone = persistent_settings.clone();
    thumbnail_window.on_move_selection(move |delta| {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            let mut app = app_state_clone.borrow_mut();
            if let Some(index) = app.current_image_index {
                let new_index = (index as i32 + delta).max(0).min(app.image_list.len() as i32 - 1) as usize;
                if new_index != index {
                    let path = app.image_list[new_index].clone();
                    set_image(&ui, &thumb_ui, &mut app, path, &volatile_settings_clone, &persistent_settings_clone);
                }
            }
        }
    });

    let thumbnail_window_handle = thumbnail_window.as_weak();
    let volatile_settings_clone_for_thumb = volatile_settings.clone();
    main_window.on_show_thumbnail_window(move || {
        if let Some(thumb_ui) = thumbnail_window_handle.upgrade() {
            let is_visible = thumb_ui.window().is_visible();
            if is_visible {
                let _ = thumb_ui.hide();
                volatile_settings_clone_for_thumb.borrow_mut().thumbnail_window_visible = false;
            } else {
                let _ = thumb_ui.show();
                volatile_settings_clone_for_thumb.borrow_mut().thumbnail_window_visible = true;
            }
            let _ = volatile_settings_clone_for_thumb.borrow().save_blocking();
        }
    });

    let main_window_handle = main_window.as_weak();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone_for_zoom = persistent_settings.clone();
    main_window.on_zoom_image(move |delta_y, mouse_x, mouse_y| {
        if let Some(ui) = main_window_handle.upgrade() {
            let mut volatile = volatile_settings_clone.borrow_mut();
            let persistent = persistent_settings_clone_for_zoom.borrow();
            let old_scale = volatile.image_scale;

            let zoom_val = persistent.zoom_value as f64;
            let new_scale = if delta_y < 0.0 {
                match persistent.zoom_mode {
                    voutil::settings::ZoomMode::Multiplier => old_scale * (1.0 + zoom_val),
                    voutil::settings::ZoomMode::Additive => old_scale + zoom_val,
                }
            } else {
                match persistent.zoom_mode {
                    voutil::settings::ZoomMode::Multiplier => old_scale / (1.0 + zoom_val),
                    voutil::settings::ZoomMode::Additive => old_scale - zoom_val,
                }
            }
            .max(0.1)
            .min(10.0);

            let old_image_x = ui.get_image_x() as f64;
            let old_image_y = ui.get_image_y() as f64;
            let mouse_img_x = (mouse_x as f64 - old_image_x) / old_scale;
            let mouse_img_y = (mouse_y as f64 - old_image_y) / old_scale;
            volatile.image_scale = new_scale;
            ui.set_image_scale(new_scale as f32);
            ui.set_image_x((mouse_x as f64 - mouse_img_x * new_scale).round() as i32);
            ui.set_image_y((mouse_y as f64 - mouse_img_y * new_scale).round() as i32);
            update_image_info(&ui);
        }
    });

    let v_settings_scale = volatile_settings.clone();
    main_window.on_scale_changed(move |new_scale| {
        v_settings_scale.borrow_mut().image_scale = new_scale as f64;
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    main_window.on_check_selection_hit(move |x, y| {
        if let Some(ui) = main_window_handle.upgrade() {
            let app_state = app_state_clone.borrow();
            if let Some(selection) = app_state.selection {
                let (image_x, image_y, scale) = (
                    ui.get_image_x() as f32,
                    ui.get_image_y() as f32,
                    ui.get_image_scale(),
                );
                let img_coord_x = ((x - image_x) / scale).round() as u32;
                let img_coord_y = ((y - image_y) / scale).round() as u32;
                let tolerance = (10.0 / scale) as u32;
                if selection
                    .get_resize_handle(img_coord_x, img_coord_y, tolerance)
                    .is_some()
                    || selection.contains(img_coord_x, img_coord_y)
                {
                    return true;
                }
            }
        }
        false
    });

    let main_window_handle = main_window.as_weak();
    let app_state_clone = app_state.clone();
    let persistent_settings_for_pointer = persistent_settings.clone();
    main_window.on_pointer_event(move |event_type, _event_button, x, y| {
        if let Some(ui) = main_window_handle.upgrade() {
            let mut app_state = app_state_clone.borrow_mut();
            if let Some(pixel_buffer) = ui.get_image_display().to_rgba8() {
                let (image_x, image_y, scale, img_w, img_h) = (
                    ui.get_image_x() as f32,
                    ui.get_image_y() as f32,
                    ui.get_image_scale(),
                    pixel_buffer.width(),
                    pixel_buffer.height(),
                );
                let (unclamped_img_coord_x, unclamped_img_coord_y) = (
                    ((x - image_x) / scale) as i32,
                    ((y - image_y) / scale) as i32,
                );

                match event_type.as_str() {
                    "down" => {
                        let (img_coord_x, img_coord_y) = (
                            unclamped_img_coord_x.max(0).min(img_w as i32 - 1) as u32,
                            unclamped_img_coord_y.max(0).min(img_h as i32 - 1) as u32,
                        );
                        let tolerance = (10.0 / scale) as u32;
                        if let Some(selection) = app_state.selection {
                            if let Some(handle) =
                                selection.get_resize_handle(img_coord_x, img_coord_y, tolerance)
                            {
                                app_state.drag_mode = DragMode::ResizingSelection(handle);
                                app_state.initial_selection_on_drag = Some(selection);
                                app_state.initial_mouse_on_drag = Some((img_coord_x, img_coord_y));
                                return;
                            } else if selection.contains(img_coord_x, img_coord_y) {
                                app_state.drag_mode = DragMode::MovingSelection;
                                app_state.initial_selection_on_drag = Some(selection);
                                app_state.initial_mouse_on_drag = Some((img_coord_x, img_coord_y));
                                return;
                            }
                        }
                        app_state.drag_mode = DragMode::Selecting;
                        app_state.selection = None;
                        app_state.selection_start_point = Some((img_coord_x, img_coord_y));
                    }
                    "move" => {
                        let (img_coord_x, img_coord_y) = (
                            unclamped_img_coord_x.max(0).min(img_w as i32 - 1) as u32,
                            unclamped_img_coord_y.max(0).min(img_h as i32 - 1) as u32,
                        );
                        match app_state.drag_mode {
                            DragMode::Selecting => {
                                if let Some(start_point) = app_state.selection_start_point {
                                    // Set the mouse coordinates as a tentative endpoint.
                                    let mut x1 = start_point.0 as i32;
                                    let mut y1 = start_point.1 as i32;
                                    let mut x2 = unclamped_img_coord_x;
                                    let mut y2 = unclamped_img_coord_y;

                                    // Adjust the endpoint to apply the aspect ratio.
                                    let settings = persistent_settings_for_pointer.borrow();
                                    if settings.crop_aspect_ratio != "Free" {
                                        if let Some(ratio) = settings.crop_aspect_ratio.split_once(':')
                                            .and_then(|(a, b)| Some(a.parse::<f32>().ok()? / b.parse::<f32>().ok()?))
                                        {
                                            let dx = x2 - x1;
                                            let dy = y2 - y1;
                                            if dx.abs() as f32 / dy.abs().max(1) as f32 > ratio {
                                                y2 = y1 + ( (dx.abs() as f32 / ratio).round() as i32 * (if dy == 0 {1} else {dy.signum()}) );
                                            } else {
                                                x2 = x1 + ( (dy.abs() as f32 * ratio).round() as i32 * (if dx == 0 {1} else {dx.signum()}) );
                                            }
                                        }
                                    }

                                    // Clamp the coordinates exactly as in Free mode.
                                    x1 = x1.clamp(0, img_w as i32);
                                    y1 = y1.clamp(0, img_h as i32);
                                    x2 = x2.clamp(0, img_w as i32);
                                    y2 = y2.clamp(0, img_h as i32);

                                    // Re-apply the aspect ratio, as clamping may have broken it.
                                    if settings.crop_aspect_ratio != "Free" {
                                        if let Some(ratio) = settings.crop_aspect_ratio.split_once(':')
                                            .and_then(|(a, b)| Some(a.parse::<f32>().ok()? / b.parse::<f32>().ok()?))
                                        {
                                            let dx = x2 - x1;
                                            let dy = y2 - y1;
                                            let current_w = (x2 - x1).abs();
                                            let current_h = (y2 - y1).abs();
                                            if current_w as f32 / current_h.max(1) as f32 > ratio {
                                                // Width is too wide -> adjust width to match height.
                                                let new_w = (current_h as f32 * ratio).round() as i32;
                                                if dx > 0 {
                                                    x2 = x1 + new_w;
                                                } else {
                                                    x2 = x1 - new_w;
                                                }
                                            } else {
                                                // Height is too tall -> adjust height to match width.
                                                let new_h = (current_w as f32 / ratio).round() as i32;
                                                if dy > 0 {
                                                    y2 = y1 + new_h;
                                                } else { 
                                                    y2 = y1 - new_h;
                                                }
                                            }
                                        }
                                    }

                                    // Organize coordinates and determine the final rectangle.
                                    if x1 > x2 { std::mem::swap(&mut x1, &mut x2); }
                                    if y1 > y2 { std::mem::swap(&mut y1, &mut y2); }

                                    app_state.selection = Some(SelectionRect {
                                        x: x1 as u32,
                                        y: y1 as u32,
                                        w: (x2 - x1) as u32,
                                        h: (y2 - y1) as u32,
                                    });
                                }
                            }
                            DragMode::MovingSelection => {
                                if let (Some(initial_selection), Some(initial_mouse)) = (
                                    app_state.initial_selection_on_drag,
                                    app_state.initial_mouse_on_drag,
                                ) {
                                    let delta_x = unclamped_img_coord_x - initial_mouse.0 as i32;
                                    let delta_y = unclamped_img_coord_y - initial_mouse.1 as i32;
                                    app_state.selection = Some(SelectionRect {
                                        x: ((initial_selection.x as i32 + delta_x).max(0) as u32)
                                            .min(img_w.saturating_sub(initial_selection.w)),
                                        y: ((initial_selection.y as i32 + delta_y).max(0) as u32)
                                            .min(img_h.saturating_sub(initial_selection.h)),
                                        ..initial_selection
                                    });
                                }
                            }
                            DragMode::ResizingSelection(handle) => {
                                if let (Some(initial_selection), Some(initial_mouse)) = (
                                    app_state.initial_selection_on_drag,
                                    app_state.initial_mouse_on_drag,
                                ) {
                                    let delta_x = unclamped_img_coord_x - initial_mouse.0 as i32;
                                    let delta_y = unclamped_img_coord_y - initial_mouse.1 as i32;
                                    let mut x1 = initial_selection.x as i32;
                                    let mut y1 = initial_selection.y as i32;
                                    let mut x2 = (initial_selection.x + initial_selection.w) as i32;
                                    let mut y2 = (initial_selection.y + initial_selection.h) as i32;

                                    match handle {
                                        ResizeHandle::Left => {
                                            x1 += delta_x;
                                        }
                                        ResizeHandle::Right => {
                                            x2 += delta_x;
                                        }
                                        ResizeHandle::Top => {
                                            y1 += delta_y;
                                        }
                                        ResizeHandle::Bottom => {
                                            y2 += delta_y;
                                        }
                                        ResizeHandle::TopLeft => {
                                            x1 += delta_x;
                                            y1 += delta_y;
                                        }
                                        ResizeHandle::TopRight => {
                                            x2 += delta_x;
                                            y1 += delta_y;
                                        }
                                        ResizeHandle::BottomLeft => {
                                            x1 += delta_x;
                                            y2 += delta_y;
                                        }
                                        ResizeHandle::BottomRight => {
                                            x2 += delta_x;
                                            y2 += delta_y;
                                        }
                                    }

                                    let mut w = (x2 - x1) as u32;
                                    let mut h = (y2 - y1) as u32;

                                    let settings = persistent_settings_for_pointer.borrow();
                                    if settings.crop_aspect_ratio != "Free" {
                                        let parts: Vec<&str> = settings.crop_aspect_ratio.split(':').collect();
                                        if parts.len() == 2 {
                                            if let (Ok(rw), Ok(rh)) = (parts[0].parse::<f32>(), parts[1].parse::<f32>()) {
                                                let ratio = rw / rh;
                                                // When resizing, we usually want to follow the handle's intent.
                                                // For side handles, we force the other dimension.
                                                // For corner handles, we match the ratio based on the larger side.
                                                match handle {
                                                    ResizeHandle::Left | ResizeHandle::Right => {
                                                        h = (w as f32 / ratio).round() as u32;
                                                    }
                                                    ResizeHandle::Top | ResizeHandle::Bottom => {
                                                        w = (h as f32 * ratio).round() as u32;
                                                    }
                                                    _ => {
                                                        if w as f32 / h as f32 > ratio {
                                                            h = (w as f32 / ratio).round() as u32;
                                                        } else {
                                                            w = (h as f32 * ratio).round() as u32;
                                                        }
                                                    }
                                                }
                                                
                                                // Adjust coordinates based on handle to keep anchor fixed
                                                match handle {
                                                    ResizeHandle::Left | ResizeHandle::TopLeft | ResizeHandle::BottomLeft => {
                                                        x1 = x2 - w as i32;
                                                    }
                                                    _ => {
                                                        x2 = x1 + w as i32;
                                                    }
                                                }
                                                match handle {
                                                    ResizeHandle::Top | ResizeHandle::TopLeft | ResizeHandle::TopRight => {
                                                        y1 = y2 - h as i32;
                                                    }
                                                    _ => {
                                                        y2 = y1 + h as i32;
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Clamp the coordinates exactly as in Free mode.
                                    x1 = x1.max(0).min((img_w - 1) as i32);
                                    y1 = y1.max(0).min((img_h - 1) as i32);
                                    x2 = x2.max(1).min(img_w as i32);
                                    y2 = y2.max(1).min(img_h as i32);

                                    // Re-apply the aspect ratio, as clamping may have broken it.
                                    if settings.crop_aspect_ratio != "Free" {
                                        if let Some(ratio) = settings.crop_aspect_ratio.split_once(':')
                                            .and_then(|(a, b)| Some(a.parse::<f32>().ok()? / b.parse::<f32>().ok()?))
                                        {
                                            let dx = x2 - x1;
                                            let dy = y2 - y1;
                                            let current_w = (x2 - x1).abs();
                                            let current_h = (y2 - y1).abs();
                                            if current_w as f32 / current_h.max(1) as f32 > ratio {
                                                // Width is too wide -> adjust width to match height.
                                                let new_w = (current_h as f32 * ratio).round() as i32;
                                                if dx > 0 {
                                                    x2 = x1 + new_w;
                                                } else {
                                                    x2 = x1 - new_w;
                                                }
                                            } else {
                                                // Height is too tall -> adjust height to match width.
                                                let new_h = (current_w as f32 / ratio).round() as i32;
                                                if dy > 0 {
                                                    y2 = y1 + new_h;
                                                } else { 
                                                    y2 = y1 - new_h;
                                                }
                                            }
                                        }
                                    }

                                    // Organize coordinates and determine the final rectangle.
                                    if x1 > x2 { std::mem::swap(&mut x1, &mut x2); }
                                    if y1 > y2 { std::mem::swap(&mut y1, &mut y2); }

                                    app_state.selection = Some(SelectionRect {
                                        x: x1 as u32,
                                        y: y1 as u32,
                                        w: (x2 - x1).max(1) as u32,
                                        h: (y2 - y1).max(1) as u32,
                                    });
                                }
                            }
                            DragMode::None => {
                                if let Some(selection) = app_state.selection {
                                    let tolerance = (10.0 / scale) as u32;
                                    if let Some(handle) = selection.get_resize_handle(
                                        img_coord_x,
                                        img_coord_y,
                                        tolerance,
                                    ) {
                                        ui.set_cursor_handle_type(match handle {
                                            ResizeHandle::Top | ResizeHandle::Bottom => {
                                                "ns-resize".into()
                                            }
                                            ResizeHandle::Left | ResizeHandle::Right => {
                                                "ew-resize".into()
                                            }
                                            ResizeHandle::TopLeft | ResizeHandle::BottomRight => {
                                                "nwse-resize".into()
                                            }
                                            ResizeHandle::TopRight | ResizeHandle::BottomLeft => {
                                                "nesw-resize".into()
                                            }
                                        });
                                    } else if selection.contains(img_coord_x, img_coord_y) {
                                        ui.set_cursor_handle_type("move".into());
                                    } else {
                                        ui.set_cursor_handle_type("default".into());
                                    }
                                } else {
                                    ui.set_cursor_handle_type("default".into());
                                }
                            }
                        }
                    }
                    "up" => {
                        if let Some(selection) = app_state.selection {
                            if selection.w < 10 || selection.h < 10 {
                                app_state.selection = None;
                            }
                        }
                        app_state.drag_mode = DragMode::None;
                        app_state.selection_start_point = None;
                        app_state.initial_selection_on_drag = None;
                        app_state.initial_mouse_on_drag = None;
                    }
                    _ => {}
                }
            }
        }
    });

    // --- Tick handler for dynamic resize/move ---
    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let settings_window_handle_for_tick = settings_window.as_weak();
    let color_correction_ui_for_tick = color_correction_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_for_tick = persistent_settings.clone();
    main_window.on_tick(move |auto_fit| {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            let mut app_state = app_state_clone.borrow_mut();
            let mut volatile = volatile_settings_clone.borrow_mut();

            let scale_factor = ui.window().scale_factor();
            let current_pos = slint::LogicalPosition::from_physical(ui.window().position(), scale_factor);
            let current_size = slint::LogicalSize::from_physical(ui.window().size(), scale_factor);

            // Only save position and size if not in fullscreen mode
            if !ui.get_fullscreen_enabled() {
                if app_state.last_window_position != current_pos {
                    volatile.window_position = current_pos;
                    let _ = volatile.save_blocking();
                    app_state.last_window_position = current_pos;
                }

                if app_state.last_window_size != current_size {
                    volatile.window_size = current_size;
                    let _ = volatile.save_blocking();
                    app_state.last_window_size = current_size;
                    // Trigger auto-fit when window size changes
                    ui.set_auto_fit(auto_fit);
                }
            } else {
                // If in fullscreen, just update app_state.last_window_position/size
                // but don't save to volatile settings, preserving the last non-fullscreen state.
                app_state.last_window_position = current_pos;
                app_state.last_window_size = current_size;
            }

            if thumb_ui.window().is_visible() {
                let thumb_scale_factor = thumb_ui.window().scale_factor();
                let thumb_current_pos = slint::LogicalPosition::from_physical(thumb_ui.window().position(), thumb_scale_factor);
                if app_state.last_thumbnail_window_position != thumb_current_pos {
                    volatile.thumbnail_window_position = thumb_current_pos;
                    let _ = volatile.save_blocking();
                    app_state.last_thumbnail_window_position = thumb_current_pos;
                }

                let thumb_current_size = slint::LogicalSize::from_physical(thumb_ui.window().size(), thumb_scale_factor);
                if app_state.last_thumbnail_window_size != thumb_current_size {
                    volatile.thumbnail_window_size = thumb_current_size;
                    let _ = volatile.save_blocking();
                    app_state.last_thumbnail_window_size = thumb_current_size;
                }
            }

            // Periodic theme check for "System" mode (every 60 ticks, ~1 second)
            app_state.tick_count += 1;
            if app_state.tick_count % 60 == 0 {
                let theme_choice = persistent_settings_for_tick.borrow().theme.clone();
                if theme_choice == voutil::settings::ColorTheme::System {
                    if let (Some(sw), Some(ccw)) = (
                        settings_window_handle_for_tick.upgrade(),
                        color_correction_ui_for_tick.upgrade(),
                    ) {
                        let accent = persistent_settings_for_tick.borrow().accent_color;
                        apply_ui_theme(&ui, &sw, &thumb_ui, &ccw, theme_choice, accent);
                    }
                }
            }

            update_image_info(&ui);
            ui.set_current_image_index(app_state.current_image_index.map(|i| i as i32).unwrap_or(-1));
            ui.set_total_images(app_state.image_list.len() as i32);

            // Process newly arrived thumbnails
            if let Some(ref rx) = app_state.thumbnail_receiver {
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        ThumbnailMessage::Data(buffer, path_string) => {
                            let thumbnails_model = thumb_ui.get_thumbnails();
                            let thumbnails = thumbnails_model
                                .as_any()
                                .downcast_ref::<VecModel<Thumbnail>>()
                                .unwrap();
                            thumbnails.push(Thumbnail {
                                source: buffer_to_slint_image(buffer),
                                path: path_string.into(),
                            });
                        }
                        ThumbnailMessage::Completion => {
                            let image_list_borrow = app_state.image_list.clone();
                            let thumbnails_model = thumb_ui.get_thumbnails();
                            let current_thumbnails = thumbnails_model
                                .as_any()
                                .downcast_ref::<VecModel<Thumbnail>>()
                                .unwrap();

                            // Create a map from path string to Thumbnail for efficient lookup
                            let mut thumbnail_map: HashMap<String, Thumbnail> = HashMap::new();
                            for i in 0..current_thumbnails.row_count() {
                                if let Some(thumb) = current_thumbnails.row_data(i) {
                                    thumbnail_map.insert(thumb.path.to_string(), thumb);
                                }
                            }

                            // Create a new VecModel with sorted thumbnails
                            let sorted_vec_model = VecModel::default();
                            for path_buf in image_list_borrow.iter() {
                                let path_string = path_buf.to_string_lossy().to_string();
                                if let Some(thumb) = thumbnail_map.remove(&path_string) {
                                    sorted_vec_model.push(thumb);
                                }
                            }
                            thumb_ui.set_thumbnails(Rc::new(sorted_vec_model).into());
                        }
                    }
                }
            }

            if let Some(selection) = app_state.selection {
                let image_x = ui.get_image_x() as f32;
                let image_y = ui.get_image_y() as f32;
                let scale = ui.get_image_scale();
                ui.set_selection_visible(true);
                ui.set_selection_x((selection.x as f32 * scale) + image_x);
                ui.set_selection_y((selection.y as f32 * scale) + image_y);
                ui.set_selection_w(selection.w as f32 * scale);
                ui.set_selection_h(selection.h as f32 * scale);
            } else {
                ui.set_selection_visible(false);
            }
        }
    });

    // --- Thumbnails window callbacks ---
    let thumbnail_window_handle = thumbnail_window.as_weak();
    thumbnail_window.on_hide(move || {
        if let Some(thumb_ui) = thumbnail_window_handle.upgrade() {
            if thumb_ui.window().is_visible() {
                let _ = thumb_ui.hide();
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let app_state_clone = app_state.clone();
    let volatile_settings_clone = volatile_settings.clone();
    let persistent_settings_clone = persistent_settings.clone();
    thumbnail_window.on_thumbnail_clicked(move |path_str| {
        if let (Some(ui), Some(thumb_ui)) = (
            main_window_handle.upgrade(),
            thumbnail_window_handle.upgrade(),
        ) {
            set_image(
                &ui,
                &thumb_ui,
                &mut app_state_clone.borrow_mut(),
                PathBuf::from(path_str.to_string()),
                &volatile_settings_clone,
                &persistent_settings_clone,
            );
        }
    });

    // --- Settings window callbacks ---
    let settings_clone = persistent_settings.clone();
    let main_ui_for_theme = main_window.as_weak();
    let settings_ui_for_theme = settings_window.as_weak();
    let thumb_ui_for_theme = thumbnail_window.as_weak();
    let color_correction_ui_for_theme = color_correction_window.as_weak();
    settings_window.on_theme_changed(move |choice_str| {
        if let (Some(ui), Some(sw), Some(tw), Some(ccw)) = (
            main_ui_for_theme.upgrade(),
            settings_ui_for_theme.upgrade(),
            thumb_ui_for_theme.upgrade(),
            color_correction_ui_for_theme.upgrade(),
        ) {
            let choice = match choice_str.as_str() {
                "Light" => voutil::settings::ColorTheme::Light,
                "System" => voutil::settings::ColorTheme::System,
                _ => voutil::settings::ColorTheme::Dark,
            };

            let mut settings = settings_clone.borrow_mut();
            settings.theme = choice.clone();
            let _ = settings.save_blocking();
            let accent = settings.accent_color;

            apply_ui_theme(&ui, &sw, &tw, &ccw, choice, accent);
            }
            });
    let settings_clone = persistent_settings.clone();
    let main_window_handle = main_window.as_weak();
    settings_window.on_show_status_messages_changed(move |enabled| {
        let mut settings = settings_clone.borrow_mut();
        settings.show_status_messages = enabled;
        let _ = settings.save_blocking();
        if let Some(ui) = main_window_handle.upgrade() {
            ui.set_show_status_messages(enabled);
        }
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_reopen_last_image_changed(move |enabled| {
        let mut settings = settings_clone.borrow_mut();
        settings.reopen_last_image = enabled;
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_use_os_sorting_changed(move |enabled| {
        let mut settings = settings_clone.borrow_mut();
        settings.use_os_sorting = enabled;
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_sort_criteria_changed(move |val| {
        let mut settings = settings_clone.borrow_mut();
        settings.sort_criteria = val.to_string();
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_sort_order_changed(move |val| {
        let mut settings = settings_clone.borrow_mut();
        settings.sort_order = val.to_string();
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_crop_aspect_ratio_changed(move |val| {
        let mut settings = settings_clone.borrow_mut();
        settings.crop_aspect_ratio = val.to_string();
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_default_save_format_changed(move |val| {
        let mut settings = settings_clone.borrow_mut();
        settings.default_save_format = val.to_string();
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_jpeg_quality_changed(move |val| {
        let mut settings = settings_clone.borrow_mut();
        settings.jpeg_quality = val as u32;
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    let main_ui_for_accent = main_window.as_weak();
    let settings_ui_for_accent = settings_window.as_weak();
    let thumb_ui_for_accent = thumbnail_window.as_weak();
    let cc_ui_for_accent = color_correction_window.as_weak();
    let settings_ui_for_accent_hex = settings_window.as_weak();
    settings_window.on_accent_color_changed(move |r, g, b| {
        let mut settings = settings_clone.borrow_mut();
        settings.accent_color = [r as u8, g as u8, b as u8];
        let _ = settings.save_blocking();
        
        if let Some(sw) = settings_ui_for_accent_hex.upgrade() {
            sw.set_accent_hex(format!("#{:02X}{:02X}{:02X}", r, g, b).into());
        }

        if let (Some(ui), Some(sw), Some(tw), Some(ccw)) = (
            main_ui_for_accent.upgrade(),
            settings_ui_for_accent.upgrade(),
            thumb_ui_for_accent.upgrade(),
            cc_ui_for_accent.upgrade(),
        ) {
            let theme = settings.theme.clone();
            apply_ui_theme(&ui, &sw, &tw, &ccw, theme, settings.accent_color);
        }
    });

    let settings_clone = persistent_settings.clone();
    let main_ui_for_accent_hex_callback = main_window.as_weak();
    let settings_ui_for_accent_hex_callback = settings_window.as_weak();
    let thumb_ui_for_accent_hex_callback = thumbnail_window.as_weak();
    let cc_ui_for_accent_hex_callback = color_correction_window.as_weak();
    settings_window.on_accent_hex_changed(move |hex| {
        let hex = hex.trim().trim_start_matches('#');
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                let mut settings = settings_clone.borrow_mut();
                settings.accent_color = [r, g, b];
                let _ = settings.save_blocking();

                if let Some(sw) = settings_ui_for_accent_hex_callback.upgrade() {
                    sw.set_accent_r(r as i32);
                    sw.set_accent_g(g as i32);
                    sw.set_accent_b(b as i32);
                    sw.set_accent_hex(format!("#{:02X}{:02X}{:02X}", r, g, b).into());
                }

                if let (Some(ui), Some(sw), Some(tw), Some(ccw)) = (
                    main_ui_for_accent_hex_callback.upgrade(),
                    settings_ui_for_accent_hex_callback.upgrade(),
                    thumb_ui_for_accent_hex_callback.upgrade(),
                    cc_ui_for_accent_hex_callback.upgrade(),
                ) {
                    let theme = settings.theme.clone();
                    apply_ui_theme(&ui, &sw, &tw, &ccw, theme, settings.accent_color);
                }
            }
        }
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_zoom_mode_changed(move |val| {
        let mut settings = settings_clone.borrow_mut();
        settings.zoom_mode = match val.as_str() {
            "Multiplier" => voutil::settings::ZoomMode::Multiplier,
            "Additive" => voutil::settings::ZoomMode::Additive,
            _ => voutil::settings::ZoomMode::Multiplier,
        };
        let _ = settings.save_blocking();
    });

    let settings_clone = persistent_settings.clone();
    settings_window.on_zoom_value_changed(move |val| {
        let mut settings = settings_clone.borrow_mut();
        settings.zoom_value = val;
        let _ = settings.save_blocking();
    });

    let main_window_handle_for_cache = main_window.as_weak();
    settings_window.on_clear_cache(move || {
        if let Ok(cache_dir) = cache::get_cache_dir() {
            let _ = fs::remove_dir_all(&cache_dir);
            let _ = fs::create_dir_all(&cache_dir);
            if let Some(ui) = main_window_handle_for_cache.upgrade() {
                ui.set_status_text("Thumbnail cache cleared.".into());
            }
        }
    });

    let main_window_handle = main_window.as_weak();
    let thumbnail_window_handle = thumbnail_window.as_weak();
    let volatile_settings_clone = volatile_settings.clone();
    let app_state_clone = app_state.clone();
    settings_window.on_reset_window_geometry(move || {
        if let (Some(ui), Some(thumb_ui)) = (main_window_handle.upgrade(), thumbnail_window_handle.upgrade()) {
            let defaults = VolatileSettings::default();
            
            ui.window().set_position(defaults.window_position);
            ui.window().set_size(defaults.window_size);
            thumb_ui.window().set_position(defaults.thumbnail_window_position);
            thumb_ui.window().set_size(defaults.thumbnail_window_size);

            let mut volatile = volatile_settings_clone.borrow_mut();
            volatile.window_position = defaults.window_position;
            volatile.window_size = defaults.window_size;
            volatile.thumbnail_window_position = defaults.thumbnail_window_position;
            volatile.thumbnail_window_size = defaults.thumbnail_window_size;
            let _ = volatile.save_blocking();

            let mut app_state = app_state_clone.borrow_mut();
            app_state.last_window_position = defaults.window_position;
            app_state.last_window_size = defaults.window_size;
            app_state.last_thumbnail_window_position = defaults.thumbnail_window_position;
            app_state.last_thumbnail_window_size = defaults.thumbnail_window_size;

            ui.set_status_text("Window positions and sizes reset.".into());
        }
    });

    let settings_clone = persistent_settings.clone();
    let settings_window_handle = settings_window.as_weak();
    settings_window.on_start_recording(move |event_type_str| {
        if let Some(sw) = settings_window_handle.upgrade() {
            let settings = settings_clone.borrow();
            let mut description = event_type_str.clone();
            for event in settings.shortcuts.keys() {
                if format!("{:?}", event) == event_type_str.as_str() {
                    description = event.description().into();
                    break;
                }
            }
            sw.set_recording_event_type(event_type_str);
            sw.set_recording_description(description);
        }
    });

    let settings_window_handle = settings_window.as_weak();
    settings_window.on_stop_recording(move || {
        if let Some(sw) = settings_window_handle.upgrade() {
            sw.set_recording_event_type("".into());
            sw.set_recording_description("".into());
        }
    });

    let settings_clone = persistent_settings.clone();
    let settings_window_handle = settings_window.as_weak();
    settings_window.on_clear_shortcuts(move |event_type_str| {
        if let Some(sw) = settings_window_handle.upgrade() {
            let mut settings = settings_clone.borrow_mut();
            // Need to find which variant the string belongs to
            // This is a bit hacky but works since we used format!("{:?}", event)
            let mut target_event = None;
            for event in settings.shortcuts.keys() {
                if format!("{:?}", event) == event_type_str.as_str() {
                    target_event = Some(event.clone());
                    break;
                }
            }
            if let Some(event) = target_event {
                settings.shortcuts.insert(event, Vec::new());
                let _ = settings.save_blocking();
                populate_shortcut_list(&sw, &settings.shortcuts);
            }
        }
    });

    let settings_clone = persistent_settings.clone();
    let settings_window_handle_for_reset = settings_window.as_weak();
    settings_window.on_reset_shortcuts(move || {
        if let Some(sw) = settings_window_handle_for_reset.upgrade() {
            let mut settings = settings_clone.borrow_mut();
            settings.shortcuts = voutil::shortcuts::ShortcutExt::default_keys();
            let _ = settings.save_blocking();
            populate_shortcut_list(&sw, &settings.shortcuts);
        }
    });

    let settings_window_handle_for_esc = settings_window.as_weak();
    settings_window.on_shortcut_pressed(move |text, _ctrl, _alt, _shift| {
        if text == slint::SharedString::from(slint::platform::Key::Escape) {
            if let Some(sw) = settings_window_handle_for_esc.upgrade() {
                let _ = sw.hide();
            }
            return true;
        }
        false
    });

    let cc_window_handle_for_esc = color_correction_window.as_weak();
    color_correction_window.on_shortcut_pressed(move |text, _ctrl, _alt, _shift| {
        if text == slint::SharedString::from(slint::platform::Key::Escape) {
            if let Some(ccw) = cc_window_handle_for_esc.upgrade() {
                let _ = ccw.hide();
            }
            return true;
        }
        false
    });

    // --- Load last image on startup if enabled ---
    if persistent_settings.borrow().reopen_last_image {
        let last_image_path = volatile_settings.borrow().last_image_path.clone();
        if last_image_path.is_file() {
             // We need to clone the handles again for this block
            let main_window_handle = main_window.as_weak();
            let thumbnail_window_handle = thumbnail_window.as_weak();
            let app_state_clone = app_state.clone();
            let volatile_settings_clone = volatile_settings.clone();
            let persistent_settings_clone = persistent_settings.clone();
            if let (Some(ui), Some(thumb_ui)) = (main_window_handle.upgrade(), thumbnail_window_handle.upgrade()) {
                set_image(
                    &ui,
                    &thumb_ui,
                    &mut app_state_clone.borrow_mut(),
                    last_image_path,
                    &volatile_settings_clone,
                    &persistent_settings_clone,
                );
            }
        }
    }

    main_window.run()
}
