//! The bar's date and time: `ddd dd. MMM  HH:mm` in the language of `LC_TIME` with the CLDR abbreviations
//! Qt uses (German "Di. 20. Okt.", otherwise English "Tue 20. Oct").
use crate::i18n::{Language, language_code};

/// A moment in the local time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// 0 is Sunday.
    pub weekday: u32,
    pub day: u32,
    /// 1 is January.
    pub month: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl LocalTime {
    pub fn now() -> Option<Self> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        let time = libc::time_t::try_from(now).ok()?;
        // SAFETY: `tm` is plain data that localtime_r fills; both pointers are valid for the call.
        let tm = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&time, &mut tm).is_null() {
                return None;
            }
            tm
        };
        let field = |value: libc::c_int| u32::try_from(value).ok();
        Some(Self {
            weekday: field(tm.tm_wday)?,
            day: field(tm.tm_mday)?,
            month: field(tm.tm_mon)? + 1,
            hour: field(tm.tm_hour)?,
            minute: field(tm.tm_min)?,
            second: field(tm.tm_sec)?.min(59),
        })
    }

    /// Seconds until the next minute begins (when the bar's text changes).
    pub fn seconds_to_next_minute(&self) -> u32 {
        60 - self.second
    }
}

/// The language of dates and times: `LC_ALL`, `LC_TIME` or `LANG`.
pub fn time_language() -> Language {
    let locale = ["LC_ALL", "LC_TIME", "LANG"].into_iter().find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
    match locale.as_deref().map(language_code) {
        Some("de") => Language::German,
        _ => Language::English,
    }
}

const DAYS_DE: [&str; 7] = ["So.", "Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa."];
const MONTHS_DE: [&str; 12] = ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."];
const DAYS_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The bar's clock text for `time`.
pub fn bar_text(time: &LocalTime, language: Language) -> String {
    let (days, months) = match language {
        Language::German => (DAYS_DE, MONTHS_DE),
        Language::English => (DAYS_EN, MONTHS_EN),
    };
    let day = days[(time.weekday % 7) as usize];
    let month = months[(time.month.clamp(1, 12) - 1) as usize];
    format!("{day} {:02}. {month}  {:02}:{:02}", time.day, time.hour, time.minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_like_qts() {
        let time = LocalTime { weekday: 2, day: 20, month: 10, hour: 19, minute: 5, second: 30 };
        assert_eq!(bar_text(&time, Language::German), "Di. 20. Okt.  19:05");
        assert_eq!(bar_text(&time, Language::English), "Tue 20. Oct  19:05");
        let time = LocalTime { weekday: 0, day: 1, month: 3, hour: 0, minute: 0, second: 0 };
        assert_eq!(bar_text(&time, Language::German), "So. 01. März  00:00");
        assert_eq!(time.seconds_to_next_minute(), 60);
    }

    #[test]
    fn now_is_a_valid_time() {
        let now = LocalTime::now().expect("local time");
        assert!(now.weekday < 7 && (1..=31).contains(&now.day) && (1..=12).contains(&now.month));
        assert!(now.hour < 24 && now.minute < 60 && now.second < 60);
    }
}
