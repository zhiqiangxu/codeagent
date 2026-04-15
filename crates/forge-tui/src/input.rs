/// 终端输入缓冲区，管理文本内容和光标位置。
#[derive(Debug, Default)]
pub struct InputBuffer {
    buf: String,
    cursor: usize,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn content(&self) -> &str {
        &self.buf
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 在光标位置插入字符。
    pub fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// 删除光标前一个字符。
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // 找到前一个字符的边界
        let prev = self.buf[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.buf.remove(prev);
        self.cursor = prev;
    }

    /// 光标左移一个字符。
    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.buf[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    /// 光标右移一个字符。
    pub fn cursor_right(&mut self) {
        if self.cursor < self.buf.len() {
            self.cursor += self.buf[self.cursor..].chars().next().map_or(0, |c| c.len_utf8());
        }
    }

    /// 提交当前内容，返回文本并清空缓冲区。
    pub fn submit(&mut self) -> String {
        let text = std::mem::take(&mut self.buf);
        self.cursor = 0;
        text
    }

    /// 插入换行符（Shift+Enter）。
    pub fn newline(&mut self) {
        self.insert('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_insert_char() {
        let mut buf = InputBuffer::new();
        buf.insert('a');
        assert_eq!(buf.content(), "a");
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn test_input_backspace() {
        let mut buf = InputBuffer::new();
        buf.insert('a');
        buf.insert('b');
        assert_eq!(buf.cursor(), 2);
        buf.backspace();
        assert_eq!(buf.content(), "a");
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn test_input_backspace_empty() {
        let mut buf = InputBuffer::new();
        buf.backspace(); // 不应 panic
        assert_eq!(buf.content(), "");
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_input_cursor_left() {
        let mut buf = InputBuffer::new();
        buf.insert('a');
        buf.insert('b');
        assert_eq!(buf.cursor(), 2);
        buf.cursor_left();
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn test_input_cursor_right_at_end() {
        let mut buf = InputBuffer::new();
        buf.insert('a');
        buf.insert('b');
        assert_eq!(buf.cursor(), 2);
        buf.cursor_right(); // 不应越界
        assert_eq!(buf.cursor(), 2);
    }

    #[test]
    fn test_input_submit() {
        let mut buf = InputBuffer::new();
        buf.insert('h');
        buf.insert('e');
        buf.insert('l');
        buf.insert('l');
        buf.insert('o');
        let text = buf.submit();
        assert_eq!(text, "hello");
        assert_eq!(buf.content(), "");
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_input_multiline() {
        let mut buf = InputBuffer::new();
        for c in "line1".chars() {
            buf.insert(c);
        }
        buf.newline();
        assert_eq!(buf.content(), "line1\n");
        assert_eq!(buf.cursor(), 6); // "line1\n".len()
    }
}
