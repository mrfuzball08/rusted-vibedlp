use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    AyuGray,
    CatppuccinMocha,
    Nord,
    Dracula,
}

impl Theme {
    const ALL: [Self; 4] = [
        Self::AyuGray,
        Self::CatppuccinMocha,
        Self::Nord,
        Self::Dracula,
    ];

    pub fn from_index(index: i32) -> Option<Self> {
        Self::ALL.get(usize::try_from(index).ok()?).copied()
    }

    pub fn index(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub destination: Option<PathBuf>,
    pub theme: Theme,
}

impl Settings {
    pub fn destination(&self) -> PathBuf {
        self.destination.clone().unwrap_or_else(system_downloads)
    }
}

pub fn system_downloads() -> PathBuf {
    if let Some(user) = UserDirs::new() {
        return user
            .download_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| user.home_dir().join("Downloads"));
    }

    env::current_dir()
        .unwrap_or_else(|_| env::temp_dir())
        .join("Downloads")
}

pub struct Store(Option<PathBuf>);

impl Store {
    pub fn new() -> Self {
        Self(
            ProjectDirs::from("app", "VibeDLP", "VibeDLP")
                .map(|project| project.config_dir().join("settings.json")),
        )
    }

    pub fn load(&self) -> io::Result<Settings> {
        match fs::read(self.path()?) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        let path = self.path()?;
        let directory = path.parent().expect("la configuración tiene un directorio");
        fs::create_dir_all(directory)?;

        let mut temporary = NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(&mut temporary, settings)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    }

    fn path(&self) -> io::Result<&Path> {
        self.0.as_deref().ok_or_else(|| {
            io::Error::other("no se encontró la carpeta de configuración del sistema")
        })
    }
}
