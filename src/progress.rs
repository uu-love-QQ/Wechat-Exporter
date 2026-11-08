use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

/// Lightweight stderr progress bar. It stays silent when stderr is redirected
/// or when `--silent` was passed, so JSON/CLI output remains clean.
pub struct Progress {
    enabled: bool,
    interactive: bool,
    last_render: Instant,
    last_width: usize,
    message_done: usize,
    message_total: usize,
    attachment_done: usize,
    attachment_total: usize,
    step_done: usize,
    step_total: usize,
    step_label: String,
    phase: String,
}

impl Progress {
    pub fn new(silent: bool) -> Self {
        let interactive = io::stderr().is_terminal();
        Self {
            enabled: !silent && interactive,
            interactive,
            last_render: Instant::now() - Duration::from_secs(10),
            last_width: 0,
            message_done: 0,
            message_total: 0,
            attachment_done: 0,
            attachment_total: 0,
            step_done: 0,
            step_total: 0,
            step_label: String::new(),
            phase: String::new(),
        }
    }

    pub fn set_totals(&mut self, messages: usize, attachments: usize) {
        self.message_total = messages;
        self.attachment_total = attachments;
        self.render(true);
    }

    pub fn set_phase(&mut self, phase: impl Into<String>) {
        self.phase = phase.into();
        self.render(true);
    }

    pub fn steps(&mut self, done: usize, total: usize, label: impl Into<String>) {
        self.step_done = done;
        self.step_total = total;
        self.step_label = label.into();
        self.render(true);
    }

    pub fn messages(&mut self, done: usize, total: usize) {
        self.message_done = done;
        self.message_total = total;
        self.render(false);
    }

    pub fn attachments(&mut self, done: usize, total: usize) {
        self.attachment_done = done;
        self.attachment_total = total;
        self.render(false);
    }

    pub fn finish(&mut self) {
        if !self.enabled {
            return;
        }
        if self.message_total > 0 {
            self.message_done = self.message_total;
        }
        if self.step_total > 0 {
            self.step_done = self.step_total;
        }
        self.render(true);
        if self.interactive {
            let _ = writeln!(io::stderr());
        }
        self.enabled = false;
    }

    fn render(&mut self, force: bool) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        if !force && now.duration_since(self.last_render) < Duration::from_millis(120) {
            return;
        }
        self.last_render = now;
        let percent = if self.step_total > 0 {
            (self.step_done * 100 / self.step_total).min(100)
        } else if self.message_total > 0 {
            (self.message_done * 100 / self.message_total).min(100)
        } else {
            0
        };
        let width = 22usize;
        let filled = width * percent / 100;
        let bar = format!("{}{}", "#".repeat(filled), "-".repeat(width - filled));
        let mut line = if self.phase.is_empty() {
            format!("[{bar}] {percent:3}%")
        } else {
            format!("{} [{bar}] {percent:3}%", self.phase)
        };
        if self.step_total > 0 {
            line.push_str(&format!(" · {} {}/{}", self.step_label, self.step_done, self.step_total));
        }
        if self.message_total > 0 {
            line.push_str(&format!(" · 消息 {}/{}", self.message_done, self.message_total));
        }
        if self.attachment_total > 0 || self.attachment_done > 0 {
            line.push_str(&format!(
                " · 附件 {}/{}",
                self.attachment_done, self.attachment_total
            ));
        }
        let padding = " ".repeat(self.last_width.saturating_sub(line.chars().count()));
        self.last_width = line.chars().count();
        let _ = write!(io::stderr(), "\r{line}{padding}");
        let _ = io::stderr().flush();
    }
}
