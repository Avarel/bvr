pub struct History {
    buf: Vec<String>,
    curr: String,
    index: usize,
}

impl History {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            curr: String::new(),
            index: 0,
        }
    }

    pub fn is_using_history(&self) -> bool {
        self.index < self.buf.len()
    }

    pub fn push(&mut self, line: String) {
        self.buf.push(line);
        self.index = self.buf.len();
    }

    pub fn set_curr(&mut self, line: String) {
        self.curr = line;
    }

    fn access(&mut self) -> Option<(&str, bool)> {
        if self.buf.is_empty() {
            return None;
        }
        self.index = self.index.min(self.buf.len());
        if self.index == self.buf.len() {
            return Some((&self.curr, false));
        } else {
            return Some((&self.buf[self.index], true));
        }
    }

    pub fn backward(&mut self) -> Option<(&str, bool)> {
        self.index = self.index.saturating_sub(1);
        self.access()
    }

    pub fn forward(&mut self) -> Option<(&str, bool)> {
        self.index = self.index.saturating_add(1);
        self.access()
    }
}
