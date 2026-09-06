use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const PREFERENCES_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemePreference {
    Light,
    Dark,
    #[default]
    System,
}

impl ThemePreference {
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum UiLanguage {
    #[default]
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en-US")]
    English,
}

impl UiLanguage {
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Chinese => Self::English,
            Self::English => Self::Chinese,
        }
    }

    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Chinese => "中文",
            Self::English => "English",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DesktopPreferences {
    schema_version: u32,
    pub(crate) theme: ThemePreference,
    pub(crate) language: UiLanguage,
    pub(crate) sidebar_visible: bool,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            schema_version: PREFERENCES_SCHEMA_VERSION,
            theme: ThemePreference::System,
            language: UiLanguage::Chinese,
            sidebar_visible: true,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NativePreferencesStore {
    path: PathBuf,
}

impl NativePreferencesStore {
    pub(crate) fn new_default() -> Result<Self, String> {
        let data_dir = super::application_data_directory()
            .ok_or_else(|| "无法确定应用数据目录".to_string())?;
        Ok(Self::new(data_dir.join("preferences.json")))
    }

    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn load(&self) -> Result<DesktopPreferences, String> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DesktopPreferences::default());
            }
            Err(error) => return Err(format!("读取原生偏好设置失败：{error}")),
        };
        let preferences: DesktopPreferences = serde_json::from_slice(&bytes)
            .map_err(|error| format!("解析原生偏好设置失败：{error}"))?;
        if preferences.schema_version != PREFERENCES_SCHEMA_VERSION {
            return Err(format!("不支持偏好设置版本 {}", preferences.schema_version));
        }
        Ok(preferences)
    }

    pub(crate) fn save(&self, preferences: &DesktopPreferences) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "偏好设置路径缺少父目录".to_string())?;
        fs::create_dir_all(parent).map_err(|error| format!("创建偏好设置目录失败：{error}"))?;

        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("创建临时偏好设置失败：{error}"))?;
        serde_json::to_writer_pretty(&mut temporary, preferences)
            .map_err(|error| format!("序列化偏好设置失败：{error}"))?;
        temporary
            .write_all(b"\n")
            .map_err(|error| format!("写入偏好设置失败：{error}"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| format!("同步偏好设置失败：{error}"))?;
        temporary
            .persist(&self.path)
            .map_err(|error| format!("替换偏好设置失败：{}", error.error))?;
        sync_parent(parent)?;
        Ok(())
    }
}

fn sync_parent(parent: &Path) -> Result<(), String> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("同步偏好设置目录失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_preferences_use_documented_defaults() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = NativePreferencesStore::new(directory.path().join("preferences.json"));

        assert_eq!(
            store.load().expect("defaults"),
            DesktopPreferences::default()
        );
    }

    #[test]
    fn preferences_round_trip_without_webview_fields() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("preferences.json");
        let store = NativePreferencesStore::new(path.clone());
        let preferences = DesktopPreferences {
            schema_version: PREFERENCES_SCHEMA_VERSION,
            theme: ThemePreference::Dark,
            language: UiLanguage::English,
            sidebar_visible: false,
        };

        store.save(&preferences).expect("save");

        assert_eq!(store.load().expect("load"), preferences);
        let raw = fs::read_to_string(path).expect("read");
        assert!(!raw.contains("localStorage"));
        assert!(!raw.contains("mcp"));
        assert!(!raw.contains("update"));
    }

    #[test]
    fn invalid_preferences_are_left_untouched() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("preferences.json");
        fs::write(&path, b"{not json").expect("seed invalid preferences");
        let store = NativePreferencesStore::new(path.clone());

        assert!(store.load().is_err());
        assert_eq!(fs::read(path).expect("read"), b"{not json");
    }
}
