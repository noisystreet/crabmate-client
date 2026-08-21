/// 界面语言（与 `<html lang>` 对齐；持久化在 **`/user-data/prefs.locale`**）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Locale {
    ZhHans,
    En,
}

impl Locale {
    pub fn from_storage_slug(s: &str) -> Self {
        match s.trim() {
            "en" => Locale::En,
            _ => Locale::ZhHans,
        }
    }

    pub fn from_html_lang(s: &str) -> Self {
        let l = s.trim().to_ascii_lowercase();
        if l == "en" || l.starts_with("en-") {
            Locale::En
        } else {
            Locale::ZhHans
        }
    }

    pub fn html_lang(self) -> &'static str {
        match self {
            Locale::ZhHans => "zh-Hans",
            Locale::En => "en",
        }
    }

    pub fn storage_slug(self) -> &'static str {
        match self {
            Locale::ZhHans => "zh-Hans",
            Locale::En => "en",
        }
    }
}

#[must_use]
pub fn load_locale_from_storage() -> Locale {
    Locale::ZhHans
}

pub fn store_locale_slug(_slug: &str) {
    // 由 [`crate::user_prefs_sync`] 写入 `/user-data/prefs`。
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_html_lang_reads_en() {
        assert_eq!(Locale::from_html_lang("en"), Locale::En);
        assert_eq!(Locale::from_html_lang("en-US"), Locale::En);
        assert_eq!(Locale::from_html_lang("zh-Hans"), Locale::ZhHans);
    }
}
