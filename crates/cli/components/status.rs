use std::{
    borrow::Cow,
    ops::Deref,
    time::{Duration, Instant},
};

pub struct StatusApp {
    message: String,
    timestamp: Option<(Instant, Duration)>,
}

impl StatusApp {
    pub const fn new() -> Self {
        Self {
            message: String::new(),
            timestamp: None,
        }
    }

    pub fn submit_message(&mut self, message: String) {
        self.submit_message_with_duration(message, Some(Duration::from_secs(2)))
    }

    pub fn submit_message_with_duration(&mut self, message: String, duration: Option<Duration>) {
        if message.is_empty() {
            self.message.clear();
            self.timestamp = None;
        } else {
            self.message = message;
            self.timestamp = duration.map(|dur| (Instant::now(), dur));
        }
    }

    pub fn get_message_update(&mut self) -> Option<Cow<str>> {
        if let Some((time, dur)) = self.timestamp {
            if time.elapsed() > dur {
                self.timestamp = None;
                let message = std::mem::take(&mut self.message);
                return Some(Cow::Owned(message));
            }
        }
        (!self.message.is_empty()).then(|| Cow::Borrowed(self.message.deref()))
    }
}
