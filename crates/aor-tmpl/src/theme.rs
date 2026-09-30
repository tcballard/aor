use std::{collections::BTreeMap, path::Path};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub background: String,
    pub foreground: String,
    pub accent: String,
    pub muted: String,
    pub border: String,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            background: "#1a1b26".into(),
            foreground: "#c0caf5".into(),
            accent: "#7aa2f7".into(),
            muted: "#a9b1d6".into(),
            border: "#414868".into(),
        }
    }
}
fn colour(s: &str) -> Option<String> {
    let s = s.trim().trim_matches(['"', '\'']);
    if s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(s.to_ascii_lowercase())
    } else {
        None
    }
}
impl Theme {
    /// The adapter accepts only hexadecimal colours, never raw CSS or shell content.
    /// Supports palette colors.toml and terminal colors from kitty.conf/alacritty.toml.
    pub fn from_directory(path: &Path) -> Self {
        for name in ["colors.toml", "kitty.conf", "alacritty.toml"] {
            let file = path.join(name);
            if !std::fs::metadata(&file).is_ok_and(|m| m.len() <= 65536) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(file) {
                if let Some(theme) = Self::parse(&text) {
                    return theme;
                }
            }
        }
        Self::default()
    }
    pub fn active() -> Self {
        std::env::var_os("HOME")
            .map(|h| Self::from_directory(&Path::new(&h).join(".config/omarchy/current/theme")))
            .unwrap_or_default()
    }
    pub fn parse(text: &str) -> Option<Self> {
        let mut entries = BTreeMap::new();
        let mut section = "";
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') {
                section = &line[1..line.len() - 1];
                continue;
            }
            let pair = line
                .split_once('=')
                .or_else(|| line.split_once(char::is_whitespace));
            if let Some((key, value)) = pair {
                if let Some(value) = colour(value) {
                    entries.insert(format!("{section}.{}", key.trim()), value.clone());
                    entries.entry(key.trim().to_owned()).or_insert(value);
                }
            }
        }
        let get = |keys: &[&str]| keys.iter().find_map(|key| entries.get(*key).cloned());
        let background = get(&["colors.primary.background", "background"])?;
        let foreground = get(&["colors.primary.foreground", "foreground"])?;
        let fallback = Self::default();
        Some(Self {
            background,
            foreground,
            accent: get(&["accent", "colors.normal.blue", "color4"]).unwrap_or(fallback.accent),
            muted: get(&["muted", "colors.bright.black", "color8"]).unwrap_or(fallback.muted),
            border: get(&["border", "colors.normal.black", "color0"]).unwrap_or(fallback.border),
        })
    }
    pub fn css(&self) -> String {
        format!(
            ":root{{--bg:{};--fg:{};--accent:{};--muted:{};--border:{}}}",
            self.background, self.foreground, self.accent, self.muted, self.border
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_mapping_and_injection_rejection() {
        let t = Theme::parse("background = '#102030'\nforeground = '#aabbcc'\naccent = '#123456'")
            .unwrap();
        assert_eq!(t.background, "#102030");
        assert!(t.css().contains("--accent:#123456"));
        assert!(
            Theme::parse("background = 'red;url(https://bad)'\nforeground = '#aabbcc'").is_none()
        );
    }
    #[test]
    fn unknown_theme_falls_back() {
        assert_eq!(
            Theme::from_directory(Path::new("/aor-does-not-exist")),
            Theme::default()
        );
    }
}
