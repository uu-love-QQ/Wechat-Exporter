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
        if self.attachment_total > 0 {
            self.attachment_done = self.attachment_total;
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
        let width = display_width(&line);
        let padding = " ".repeat(self.last_width.saturating_sub(width));
        self.last_width = width;
        let _ = write!(io::stderr(), "\r{line}{padding}");
        let _ = io::stderr().flush();
    }
}

fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

fn char_width(ch: char) -> usize {
    let code = ch as u32;
    if (0x0300..=0x036f).contains(&code)
        || (0x1ab0..=0x1aff).contains(&code)
        || (0x1dc0..=0x1dff).contains(&code)
        || (0x20d0..=0x20ff).contains(&code)
        || (0xfe20..=0xfe2f).contains(&code)
    {
        0
    } else if (0x1100..=0x115f).contains(&code)
        || (0x2e80..=0xa4cf).contains(&code)
        || (0xac00..=0xd7a3).contains(&code)
        || (0xf900..=0xfaff).contains(&code)
        || (0xfe10..=0xfe19).contains(&code)
        || (0xfe30..=0xfe6f).contains(&code)
        || (0xff00..=0xff60).contains(&code)
        || (0xffe0..=0xffe6).contains(&code)
        || (0x1f300..=0x1faff).contains(&code)
        || (0x20000..=0x3fffd).contains(&code)
    {
        2
    } else {
        1
    }
}
