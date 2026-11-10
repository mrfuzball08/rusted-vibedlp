#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod download;
mod settings;

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

use download::{AUDIO_FORMATS, Download, Outcome, Tools, Update, VIDEO_FORMATS};
use settings::{Settings, Store, Theme};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

slint::include_modules!();

enum Event {
    Update(Update),
    Finished(Result<Outcome, String>),
}

struct ActiveDownload {
    cancel: Sender<()>,
    events: Receiver<Event>,
    thread: JoinHandle<()>,
    destination: PathBuf,
}

struct AppState {
    store: Store,
    settings: Settings,
    active: Option<ActiveDownload>,
    last_destination: Option<PathBuf>,
}

impl AppState {
    fn save(&mut self, ui: &AppWindow, settings: Settings) {
        match self.store.save(&settings) {
            Ok(()) => {
                self.settings = settings;
                show_settings(ui, &self.settings);
                ui.set_settings_message("".into());
            }
            Err(error) => {
                ui.set_settings_message(
                    format!("No se pudo guardar la configuración. {error}").into(),
                );
            }
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let store = Store::new();
    let settings = store.load().unwrap_or_else(|error| {
        ui.set_settings_message(format!("No se pudo leer la configuración. {error}").into());
        Settings::default()
    });

    show_settings(&ui, &settings);
    show_tools(&ui, &Tools::detect());
    let format_labels = |formats: &[download::Format]| {
        let labels: Vec<SharedString> =
            formats.iter().map(|format| format.label().into()).collect();
        ModelRc::new(VecModel::from(labels))
    };
    ui.set_video_formats(format_labels(VIDEO_FORMATS));
    ui.set_audio_formats(format_labels(AUDIO_FORMATS));
    let state = Rc::new(RefCell::new(AppState {
        store,
        settings,
        active: None,
        last_destination: None,
    }));

    bind_settings(&ui, &state);
    bind_downloads(&ui, &state);

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), {
        let state = state.clone();
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                poll_download(&ui, &mut state.borrow_mut());
            }
        }
    });

    let result = ui.run();
    if let Some(active) = state.borrow_mut().active.take() {
        let _ = active.cancel.send(());
        let _ = active.thread.join();
    }
    result
}

fn bind_settings(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    ui.on_browse_destination({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let current = state.borrow().settings.destination();
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(&ui.window().window_handle())
                .set_title("Elegir carpeta de descarga");
            let dialog = if current.is_dir() {
                dialog.set_directory(current)
            } else {
                dialog
            };
            let picker = dialog.pick_folder();
            let state = state.clone();
            let weak = weak.clone();
            let _ = slint::spawn_local(async move {
                if let Some(folder) = picker.await
                    && let Some(ui) = weak.upgrade()
                {
                    let mut state = state.borrow_mut();
                    let settings = Settings {
                        destination: Some(folder.path().to_path_buf()),
                        ..state.settings.clone()
                    };
                    state.save(&ui, settings);
                }
            });
        }
    });

    ui.on_reset_destination({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let mut state = state.borrow_mut();
                let settings = Settings {
                    destination: None,
                    ..state.settings.clone()
                };
                state.save(&ui, settings);
            }
        }
    });

    ui.on_choose_theme({
        let weak = ui.as_weak();
        let state = state.clone();
        move |index| {
            if let (Some(ui), Some(theme)) = (weak.upgrade(), Theme::from_index(index)) {
                let mut state = state.borrow_mut();
                let settings = Settings {
                    theme,
                    ..state.settings.clone()
                };
                state.save(&ui, settings);
            }
        }
    });

    ui.on_refresh_tools({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                show_tools(&ui, &Tools::detect());
            }
        }
    });
}

fn bind_downloads(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    ui.on_start_download({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let mut state = state.borrow_mut();
                if state.active.is_none()
                    && let Err(error) = start_download(&ui, &mut state)
                {
                    show_error(&ui, &error);
                }
            }
        }
    });

    ui.on_cancel_download({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            if let Some(ui) = weak.upgrade()
                && let Some(active) = &state.borrow().active
                && active.cancel.send(()).is_ok()
            {
                ui.set_cancelling(true);
                ui.set_progress(-1.0);
                ui.set_progress_detail("Deteniendo la descarga…".into());
            }
        }
    });

    ui.on_open_destination({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            if let Some(destination) = &state.borrow().last_destination
                && let Err(error) = open::that_detached(destination)
                && let Some(ui) = weak.upgrade()
            {
                show_error(&ui, &format!("No se pudo abrir la carpeta. {error}"));
            }
        }
    });
}

fn start_download(ui: &AppWindow, state: &mut AppState) -> Result<(), String> {
    let (formats, index) = if ui.get_audio() {
        (AUDIO_FORMATS, ui.get_audio_format())
    } else {
        (VIDEO_FORMATS, ui.get_video_format())
    };
    let format = *formats
        .get(index as usize)
        .ok_or("Elige un formato de descarga válido.")?;
    let download = Download::new(
        ui.get_url().as_str(),
        state.settings.destination(),
        format,
        ui.get_playlist(),
    )?;
    let tools = Tools::detect();
    show_tools(ui, &tools);

    let destination = download.destination.clone();
    let (cancel, receiver) = mpsc::channel();
    let (events, updates) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("yt-dlp".into())
        .spawn(move || {
            let result = download::run(&download, &tools, receiver, |update| {
                let _ = events.send(Event::Update(update));
            });
            let _ = events.send(Event::Finished(result));
        })
        .map_err(|error| format!("No se pudo preparar la descarga. {error}"))?;

    ui.set_active_destination(display_path(&destination).into());
    state.active = Some(ActiveDownload {
        cancel,
        events: updates,
        thread,
        destination,
    });
    ui.set_busy(true);
    ui.set_cancelling(false);
    ui.set_result_message("".into());
    ui.set_can_open_destination(false);
    state.last_destination = None;
    ui.set_progress(-1.0);
    ui.set_progress_title("Preparando descarga".into());
    ui.set_progress_detail("Consultando el enlace con yt-dlp…".into());
    Ok(())
}

fn poll_download(ui: &AppWindow, state: &mut AppState) {
    let Some(active) = &state.active else { return };
    let result = loop {
        match active.events.try_recv() {
            Ok(Event::Update(update)) => {
                if !ui.get_cancelling() {
                    show_update(ui, update);
                }
            }
            Ok(Event::Finished(result)) => break result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                break Err("El proceso de descarga se interrumpió.".into());
            }
        }
    };

    let active = state.active.take().expect("hay una descarga activa");
    let _ = active.thread.join();
    ui.set_busy(false);
    ui.set_cancelling(false);
    match result {
        Ok(Outcome::Finished(count)) => {
            state.last_destination = Some(active.destination);
            ui.set_can_open_destination(true);
            ui.set_result_error(false);
            ui.set_result_message(
                if count == 1 {
                    "Se guardó 1 archivo.".to_owned()
                } else {
                    format!("Se guardaron {count} archivos.")
                }
                .into(),
            );
        }
        Ok(Outcome::Cancelled) => {}
        Err(error) => show_error(ui, &error),
    }
}

fn show_update(ui: &AppWindow, update: Update) {
    match update {
        Update::Item {
            title,
            position,
            count,
        } => {
            let title = match (position, count) {
                (Some(position), Some(count)) => format!("{position}/{count}  ·  {title}"),
                (Some(position), None) => format!("{position}  ·  {title}"),
                _ => title,
            };
            ui.set_progress_title(title.into());
            ui.set_progress_detail("Descargando…".into());
            ui.set_progress(-1.0);
        }
        Update::Progress {
            fraction,
            downloaded_bytes,
            total_bytes,
            bytes_per_second,
            eta_seconds,
        } => {
            ui.set_progress(fraction.map_or(-1.0, |fraction| fraction as f32));
            let mut details = Vec::new();
            if let Some(bytes) = downloaded_bytes {
                let downloaded = format_bytes(bytes);
                details.push(match total_bytes {
                    Some(total) => format!("{downloaded} / {}", format_bytes(total)),
                    None => downloaded,
                });
            }
            if let Some(speed) = bytes_per_second.filter(|speed| *speed > 0.0) {
                details.push(format!("{}/s", format_bytes(speed)));
            }
            if let Some(eta) = eta_seconds.filter(|eta| *eta > 0) {
                let time = if eta >= 3600 {
                    format!("{} h {} min", eta / 3600, (eta / 60) % 60)
                } else if eta >= 60 {
                    format!("{} min {} s", eta / 60, eta % 60)
                } else {
                    format!("{eta} s")
                };
                details.push(format!("Restan {time}"));
            }
            ui.set_progress_detail(
                if details.is_empty() {
                    "Descargando…".into()
                } else {
                    details.join("  ·  ")
                }
                .into(),
            );
        }
        Update::Processing => {
            ui.set_progress(-1.0);
            ui.set_progress_detail("Procesando el archivo…".into());
        }
    }
}

fn format_bytes(bytes: f64) -> String {
    if bytes >= 1_073_741_824.0 {
        format!("{:.1} GiB", bytes / 1_073_741_824.0)
    } else if bytes >= 1_048_576.0 {
        format!("{:.1} MiB", bytes / 1_048_576.0)
    } else if bytes >= 1024.0 {
        format!("{:.1} KiB", bytes / 1024.0)
    } else {
        format!("{bytes:.0} B")
    }
}

fn show_error(ui: &AppWindow, message: &str) {
    ui.set_result_error(true);
    ui.set_result_message(message.into());
}

fn show_tools(ui: &AppWindow, tools: &Tools) {
    ui.set_engine_ready(tools.yt_dlp.is_some());
    ui.set_ffmpeg_ready(tools.ffmpeg.is_some());
}

fn show_settings(ui: &AppWindow, settings: &Settings) {
    ui.global::<Palette>()
        .set_theme_index(settings.theme.index());
    ui.set_destination(display_path(&settings.destination()).into());
    ui.set_custom_destination(settings.destination.is_some());
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}
