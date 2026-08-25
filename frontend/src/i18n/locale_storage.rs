/// 界面语言（与 `<html lang>` 对齐；持久化在 **`/user-data/prefs.locale`**）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Locale {
    ZhHans,
    En,
}

use std::cell::RefCell;

thread_local! {
    /// 进程内当前语言缓存：由 [`set_current_locale`] / [`store_locale_slug`] 写入，
    /// 供无 Leptos 信号上下文的 API 错误路径（[`load_locale_from_storage`]）读取。
    static CURRENT_LOCALE: RefCell<Option<Locale>> = const { RefCell::new(None) };
}

/// 记录当前界面语言（语言信号变更 / 保存设置时调用）。
pub fn set_current_locale(locale: Locale) {
    CURRENT_LOCALE.with(|c| *c.borrow_mut() = Some(locale));
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
    if let Some(l) = CURRENT_LOCALE.with(|c| *c.borrow()) {
        return l;
    }
    // 冷启动 / 未设置缓存：读 `<html lang>`（由 `apply_locale_html_lang` 与语言信号同步）。
    // 仅 wasm 环境可访问 DOM；宿主单测（未设置缓存）回退 ZhHans。
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(l) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
            .and_then(|e| e.get_attribute("lang"))
            .map(|l| Locale::from_html_lang(&l))
        {
            return l;
        }
    }
    Locale::ZhHans
}

pub fn store_locale_slug(slug: &str) {
    // 由 [`crate::user_prefs_sync`] 写入 `/user-data/prefs`；此处同步进程内缓存。
    set_current_locale(Locale::from_storage_slug(slug));
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
