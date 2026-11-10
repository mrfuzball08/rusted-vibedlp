use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, BufRead, BufReader, PipeReader},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc::{Receiver, RecvTimeoutError},
    thread,
    time::Duration,
};

use command_group::CommandGroup;
use url::Url;

const ITEM: &str = "VIBEDLP_ITEM ";
const PROGRESS: &str = "VIBEDLP_PROGRESS ";
const PROCESSING: &str = "VIBEDLP_PROCESSING";
const SAVED: &str = "VIBEDLP_SAVED ";

#[derive(Clone, Copy)]
pub enum Format {
    Original,
    Video(&'static str, Option<u16>),
    Audio(&'static str, Option<u16>),
}

pub const VIDEO_FORMATS: &[Format] = &[
    Format::Original,
    Format::Video("mp4", None),
    Format::Video("mp4", Some(1080)),
    Format::Video("mp4", Some(720)),
    Format::Video("mp4", Some(480)),
    Format::Video("mkv", None),
    Format::Video("mkv", Some(1080)),
    Format::Video("mkv", Some(720)),
];

pub const AUDIO_FORMATS: &[Format] = &[
    Format::Audio("mp3", None),
    Format::Audio("mp3", Some(320)),
    Format::Audio("mp3", Some(192)),
    Format::Audio("mp3", Some(128)),
    Format::Audio("m4a", None),
    Format::Audio("opus", None),
    Format::Audio("flac", None),
];

impl Format {
    pub fn label(self) -> String {
        match self {
            Self::Original => "Automático · Mejor calidad".into(),
            Self::Video(container, height) => {
                let quality = height.map_or_else(
                    || "Mejor disponible".into(),
                    |height| format!("Hasta {height}p"),
                );
                format!("{} · {quality}", container.to_ascii_uppercase())
            }
            Self::Audio(codec, bitrate) => {
                let quality = match (codec, bitrate) {
                    (_, Some(rate)) => format!("{rate} kb/s al convertir"),
                    ("flac", None) => "Calidad de origen".into(),
                    _ => "Calidad automática".into(),
                };
                format!("{} · {quality}", codec.to_ascii_uppercase())
            }
        }
    }

    fn output_template(self) -> String {
        let suffix = match self {
            Self::Video(container, Some(height)) => format!(" [{container}-{height}p]"),
            Self::Video(container, None) => format!(" [{container}]"),
            Self::Audio(_, Some(rate)) => format!(" [{rate}k]"),
            _ => String::new(),
        };
        format!("%(title)s [%(id)s]{suffix}.%(ext)s")
    }
}

pub struct Download {
    url: String,
    pub destination: PathBuf,
    format: Format,
    playlist: bool,
}

impl Download {
    pub fn new(
        input: &str,
        destination: PathBuf,
        format: Format,
        playlist: bool,
    ) -> Result<Self, &'static str> {
        let url =
            Url::parse(input.trim()).map_err(|_| "Introduce un enlace HTTP o HTTPS válido.")?;
        if !matches!(url.scheme(), "http" | "https") || !url.has_host() {
            return Err("Introduce un enlace HTTP o HTTPS válido.");
        }

        Ok(Self {
            url: url.into(),
            destination,
            format,
            playlist,
        })
    }

    fn command(&self, tools: &Tools) -> Result<Command, String> {
        let engine = tools.yt_dlp.as_ref().ok_or(
            "No se encontró yt-dlp. Instálalo y agrégalo al PATH o colócalo junto a VibeDLP.",
        )?;
        if tools.ffmpeg.is_none() && !matches!(self.format, Format::Original) {
            return Err(
                "El formato seleccionado requiere FFmpeg. Instálalo y agrégalo al PATH.".into(),
            );
        }

        fs::create_dir_all(&self.destination)
            .map_err(|error| format!("No se pudo preparar la carpeta de destino. {error}"))?;

        let mut command = Command::new(engine);
        command
            .current_dir(&self.destination)
            .env("PATH", &tools.path)
            .env("PYTHONUTF8", "1")
            .args([
                "--ignore-config",
                "--no-colors",
                "--encoding",
                "utf-8",
                "--newline",
                "--progress",
                "--progress-delta",
                "0.35",
                "--no-simulate",
                "--socket-timeout",
                "30",
                "--windows-filenames",
                "--trim-filenames",
                "140",
                "--output-na-placeholder",
                "null",
                "--print",
                &format!("before_dl:{ITEM}[%(title)j,%(playlist_index)j,%(playlist_count)j]"),
                "--print",
                &format!("after_move:{SAVED}%(filepath)j"),
                "--progress-template",
                &format!(
                    "download:{PROGRESS}[%(progress.downloaded_bytes)j,%(progress.total_bytes)j,%(progress.total_bytes_estimate)j,%(progress.speed)j,%(progress.eta)j]"
                ),
                "--progress-template",
                &format!("postprocess:{PROCESSING}"),
                "--output",
                &self.format.output_template(),
            ]);

        if tools.node.is_some() {
            command.args(["--js-runtimes", "node"]);
        }

        if self.playlist {
            command.arg("--yes-playlist");
        } else {
            command.args(["--no-playlist", "--playlist-items", "1"]);
        }

        if let Some(ffmpeg) = &tools.ffmpeg {
            command.arg("--ffmpeg-location").arg(ffmpeg);
        }

        match self.format {
            Format::Audio(codec, bitrate) => {
                command.args([
                    "--format",
                    "bestaudio/best",
                    "--extract-audio",
                    "--audio-format",
                    codec,
                ]);
                if let Some(rate) = bitrate {
                    command.args(["--audio-quality", &format!("{rate}K")]);
                }
            }
            Format::Original => {
                let selector = if tools.ffmpeg.is_some() {
                    "bv*+ba/b"
                } else {
                    "b"
                };
                command.args(["--format", selector]);
            }
            Format::Video(container, height) => {
                let filter =
                    height.map_or_else(String::new, |height| format!("[height<={height}]"));
                command.args([
                    "--format",
                    &format!("bv*{filter}+ba/b{filter}"),
                    "--merge-output-format",
                    container,
                    "--remux-video",
                    container,
                ]);
                if container == "mp4" {
                    command.args(["--format-sort", "res,vcodec:h264,acodec:aac"]);
                }
            }
        }

        command.arg("--").arg(&self.url);

        Ok(command)
    }
}

pub struct Tools {
    path: OsString,
    pub yt_dlp: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    node: Option<PathBuf>,
}

impl Tools {
    pub fn detect() -> Self {
        let executable_dir = env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let mut directories: Vec<PathBuf> = executable_dir.into_iter().collect();
        if let Some(path) = env::var_os("PATH") {
            directories.extend(env::split_paths(&path).filter(|dir| dir.is_absolute()));
        }
        #[cfg(target_os = "macos")]
        directories.extend(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]);

        let path = env::join_paths(directories).unwrap_or_default();
        let locate = |program: &str| {
            let binary = if cfg!(windows) {
                format!("{program}.exe")
            } else {
                program.into()
            };
            which::which_in_global(binary, Some(&path)).ok()?.next()
        };

        Self {
            yt_dlp: locate("yt-dlp"),
            ffmpeg: locate("ffmpeg"),
            node: locate("node"),
            path,
        }
    }
}

pub enum Update {
    Item {
        title: String,
        position: Option<u64>,
        count: Option<u64>,
    },
    Progress {
        fraction: Option<f64>,
        downloaded_bytes: Option<f64>,
        total_bytes: Option<f64>,
        bytes_per_second: Option<f64>,
        eta_seconds: Option<u64>,
    },
    Processing,
}

pub enum Outcome {
    Finished(usize),
    Cancelled,
}

pub fn run(
    download: &Download,
    tools: &Tools,
    cancel: Receiver<()>,
    notify: impl Fn(Update) + Send,
) -> Result<Outcome, String> {
    let mut command = download.command(tools)?;
    let (reader, writer) = io::pipe().map_err(io_error)?;
    command
        .stdin(Stdio::null())
        .stdout(writer.try_clone().map_err(io_error)?)
        .stderr(writer);
    let mut group = command.group();
    #[cfg(windows)]
    group.creation_flags(0x0800_0000).kill_on_drop(true); // CREATE_NO_WINDOW
    let mut child = group
        .spawn()
        .map_err(|error| format!("No se pudo iniciar yt-dlp. {error}"))?;
    drop(command); // The reader sees EOF once yt-dlp closes its streams.

    let (exit, output) = thread::scope(|scope| {
        let output = scope.spawn(move || read_output(reader, notify));
        let exit = loop {
            match child.inner().try_wait() {
                Ok(Some(status)) => break Ok(Some(status)),
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.inner().wait();
                    break Err(error);
                }
            }

            if !matches!(
                cancel.recv_timeout(Duration::from_millis(100)),
                Err(RecvTimeoutError::Timeout)
            ) {
                match child.kill() {
                    Ok(()) => break child.inner().wait().map(|_| None),
                    Err(error) => match child.inner().try_wait() {
                        Ok(Some(status)) => break Ok(Some(status)),
                        _ => break Err(error),
                    },
                }
            }
        };
        drop(child);

        (exit, output.join().expect("el lector de yt-dlp terminó"))
    });

    let Some(status) = exit.map_err(io_error)? else {
        return Ok(Outcome::Cancelled);
    };
    let (saved, error) = output.map_err(io_error)?;
    if !status.success() {
        return Err(error.unwrap_or_else(|| describe_exit(status)));
    }
    if saved == 0 {
        return Err("yt-dlp terminó sin guardar archivos para ese enlace.".into());
    }
    Ok(Outcome::Finished(saved))
}

fn read_output(pipe: PipeReader, notify: impl Fn(Update)) -> io::Result<(usize, Option<String>)> {
    let mut reader = BufReader::new(pipe);
    let mut bytes = Vec::new();
    let mut last_error = None;
    let mut saved = 0;

    while reader.read_until(b'\n', &mut bytes)? != 0 {
        let output = String::from_utf8_lossy(&bytes);
        let line = output.trim();
        if let Some(item) = line.strip_prefix(ITEM) {
            if let Ok((title, position, count)) =
                serde_json::from_str::<(String, Option<u64>, Option<u64>)>(item)
            {
                notify(Update::Item {
                    title,
                    position,
                    count,
                });
            }
        } else if let Some(progress) = line.strip_prefix(PROGRESS) {
            type Numbers = (
                Option<f64>,
                Option<f64>,
                Option<f64>,
                Option<f64>,
                Option<u64>,
            );
            if let Ok((downloaded, total, estimate, speed, eta)) =
                serde_json::from_str::<Numbers>(progress)
            {
                let total = total.or(estimate).filter(|value| *value > 0.0);
                let fraction = downloaded
                    .zip(total)
                    .map(|(downloaded, total)| (downloaded / total).clamp(0.0, 1.0));
                notify(Update::Progress {
                    fraction,
                    downloaded_bytes: downloaded,
                    total_bytes: total,
                    bytes_per_second: speed,
                    eta_seconds: eta,
                });
            }
        } else if line == PROCESSING {
            notify(Update::Processing);
        } else if line.starts_with(SAVED) {
            saved += 1;
        } else if let Some(error) = line.strip_prefix("ERROR: ") {
            last_error = Some(error.to_owned());
        }
        bytes.clear();
    }

    Ok((saved, last_error))
}

fn io_error(error: io::Error) -> String {
    format!("No se pudo establecer comunicación con yt-dlp. {error}")
}

fn describe_exit(status: ExitStatus) -> String {
    format!("yt-dlp se cerró con un error ({status}).")
}
