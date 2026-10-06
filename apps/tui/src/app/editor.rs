//! 输入编辑器：单控件文本编辑（插入/删除/光标/历史回翻）。
//!
//! 只管文本与光标，不管渲染；多行（Alt+Enter 换行）由渲染层折行显示。

/// 输入框编辑状态。
#[derive(Debug, Default)]
pub struct Editor {
    pub text: String,
    /// 光标（字节偏移，落在字符边界）。
    cursor: usize,
    /// 已发送历史（↑ 回翻）。
    history: Vec<String>,
    hist_idx: Option<usize>,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.hist_idx = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.text.len());
        self.text.replace_range(self.cursor..next, "");
    }

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn move_right(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        self.cursor = self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.text.len());
    }

    /// 提交：取文本、清空、入历史。
    pub fn submit(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.hist_idx = None;
        if !text.trim().is_empty() {
            self.history.push(text.clone());
        }
        text
    }

    /// 历史回翻（↑）。
    pub fn prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.hist_idx {
            None => self.history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(idx);
        self.text = self.history[idx].clone();
        self.cursor = self.text.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.hist_idx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_backspace_cjk() {
        let mut e = Editor::new();
        e.insert('你');
        e.insert('好');
        e.backspace();
        assert_eq!(e.text, "你");
        e.move_left();
        e.insert('!');
        assert_eq!(e.text, "!你");
    }

    #[test]
    fn submit_history() {
        let mut e = Editor::new();
        for c in "任务A".chars() {
            e.insert(c);
        }
        assert_eq!(e.submit(), "任务A");
        e.prev();
        assert_eq!(e.text, "任务A");
    }
}
