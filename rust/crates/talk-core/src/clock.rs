//! The clock shown in front of each chat line, "HH:MM", and the first-shown time of each line.

use std::collections::HashMap;

#[cfg(windows)]
pub fn clock_text() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct LocalTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(time: *mut LocalTime);
    }
    let mut now = LocalTime::default();
    // SAFETY: GetLocalTime writes one SYSTEMTIME (eight u16 fields, the same layout as LocalTime) into the buffer it is given.
    unsafe { GetLocalTime(&mut now) };
    format!("{:02}:{:02}", now.hour, now.minute)
}

#[cfg(not(windows))]
pub fn clock_text() -> String {
    // shortcut: UTC here, because this crate has no local-time library; use the local time once one is added.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs()) % 86_400;
    format!("{:02}:{:02}", secs / 3600, secs / 60 % 60)
}

/// The time shown in front of each chat line: the time the line was first shown, kept for as long as the line is shown.
/// Keyed by the line's `seq`, so equal texts keep their own times.
#[derive(Default)]
pub struct Stamps(pub HashMap<u64, String>);

impl Stamps {
    /// The time of each line, given the `seq` of the lines still shown.
    pub fn times(&mut self, seqs: &[u64]) -> Vec<String> {
        let now = clock_text();
        let times: Vec<String> = seqs.iter().map(|seq| self.0.get(seq).cloned().unwrap_or_else(|| now.clone())).collect();
        // Only the lines still shown are kept: a line dropped from the top takes its time with it.
        self.0 = seqs.iter().zip(&times).map(|(seq, time)| (*seq, time.clone())).collect();
        times
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stamp_of_a_line_stays_while_the_line_is_shown() {
        let mut stamps = Stamps::default();
        assert_eq!(stamps.times(&[0]).len(), 1);
        stamps.0.insert(0, "00:00:01".into());
        assert_eq!(stamps.times(&[0, 1])[0], "00:00:01", "the first line keeps its time");
        stamps.0.insert(1, "00:00:02".into());
        // The first line is dropped from the top: the second one keeps its time.
        assert_eq!(stamps.times(&[1]), vec!["00:00:02".to_string()]);
    }

    #[test]
    fn equal_texts_keep_their_own_times_when_the_first_is_dropped() {
        let mut stamps = Stamps::default();
        stamps.times(&[0, 1, 2]);
        stamps.0.insert(0, "00:00:01".into());
        stamps.0.insert(1, "00:00:02".into());
        stamps.0.insert(2, "00:00:03".into());
        // Line 0 is cut from the top: the lines that stay keep 00:00:02 and 00:00:03, not the times of the first match.
        assert_eq!(stamps.times(&[1, 2]), vec!["00:00:02".to_string(), "00:00:03".to_string()]);
    }
}
