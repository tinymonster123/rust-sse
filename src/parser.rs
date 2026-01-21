/// 一条 SSE 事件（由若干行 field/value 组成，空行结束一条 message）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
    pub retry: Option<u64>,
}

impl Default for SseEvent {
    fn default() -> Self {
        Self {
            id: None,
            event: None,
            data: String::new(),
            retry: None,
        }
    }
}

/// SSE 增量解析器：输入任意切分的 bytes chunk，按 SSE 规则产出 0..N 个事件。
///
/// 设计对齐 TS 版本的 3 阶段：
/// - bytes chunk（网络分片） -> line（按 \\n/\\r/\\r\\n 切） -> message（按 field/value 组装）
pub struct SseParser {
    buffer: Vec<u8>,
    position: usize,
    discard_trailing_newline: bool, // 处理 \\r\\n 跨 chunk
    cur: SseEvent,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            position: 0,
            discard_trailing_newline: false,
            cur: SseEvent::default(),
        }
    }

    /// 喂入一个 bytes chunk，返回本次解析产生的所有事件。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        if self.buffer.is_empty() {
            self.buffer.extend_from_slice(chunk);
            self.position = 0;
        } else {
            self.buffer.extend_from_slice(chunk);
        }

        let mut out = Vec::new();
        let mut line_start = 0usize;

        while self.position < self.buffer.len() {
            // 如果上一轮遇到 \\r，下一字节若为 \\n 则吞掉
            if self.discard_trailing_newline {
                if self.buffer[self.position] == b'\n' {
                    self.position += 1;
                    line_start = self.position;
                }
                self.discard_trailing_newline = false;
            }

            // 向前寻找行尾（\\r 或 \\n）
            let mut line_end: Option<usize> = None;
            while self.position < self.buffer.len() && line_end.is_none() {
                match self.buffer[self.position] {
                    b'\r' => {
                        self.discard_trailing_newline = true;
                        line_end = Some(self.position);
                        self.position += 1;
                    }
                    b'\n' => {
                        line_end = Some(self.position);
                        self.position += 1;
                    }
                    _ => self.position += 1,
                }
            }

            let Some(end) = line_end else {
                // 到了 buffer 末尾但没找到行尾：等待下一个 chunk
                break;
            };

            let line = &self.buffer[line_start..end];
            self.on_line(line, &mut out);

            line_start = self.position; // 下一行从当前位置开始
        }

        // 丢弃已处理的前缀，保留未完成的尾部
        if line_start > 0 {
            self.buffer.drain(0..line_start);
            self.position = self.position.saturating_sub(line_start);
        }

        out
    }

    fn on_line(&mut self, line: &[u8], out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            // 空行：结束一条 message，产出事件并重置
            out.push(std::mem::take(&mut self.cur));
            self.cur = SseEvent::default();
            return;
        }

        // comment 行：以 ':' 开头
        if line[0] == b':' {
            return;
        }

        // 找到第一个 ':' 作为 field/value 分隔符
        let mut colon_idx: Option<usize> = None;
        for (i, &b) in line.iter().enumerate() {
            if b == b':' {
                colon_idx = Some(i);
                break;
            }
        }
        let Some(field_len) = colon_idx else {
            // 没有 ':'：无效行，忽略
            return;
        };
        if field_len == 0 {
            // ':' 开头已经在 comment 分支处理；这里兜底忽略
            return;
        }

        let field = &line[..field_len];
        // value 可能是 ":<value>" 或 ": <value>"
        let mut value_start = field_len + 1;
        if value_start < line.len() && line[value_start] == b' ' {
            value_start += 1;
        }
        let value = if value_start <= line.len() { &line[value_start..] } else { &[] };

        match field {
            b"data" => {
                let v = decode_utf8_lossy(value);
                if self.cur.data.is_empty() {
                    self.cur.data.push_str(&v);
                } else {
                    self.cur.data.push('\n');
                    self.cur.data.push_str(&v);
                }
            }
            b"event" => {
                self.cur.event = Some(decode_utf8_lossy(value));
            }
            b"id" => {
                // 允许空 id：语义上代表“清空 Last-Event-ID”
                self.cur.id = Some(decode_utf8_lossy(value));
            }
            b"retry" => {
                let v = decode_utf8_lossy(value);
                if let Ok(n) = v.trim().parse::<u64>() {
                    self.cur.retry = Some(n);
                }
            }
            _ => {
                // 未知字段忽略
            }
        }
    }
}

fn decode_utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_chunk_line_message_with_splits_and_crlf() {
        let mut p = SseParser::new();

        // 刻意切分：拆行、拆字段、拆 \\r\\n
        let chunks = vec![
            b"id: 1\r".as_slice(),
            b"\n",
            b": this is a comment\r\n",
            b"event: greeting\n",
            b"data: hel",
            b"lo\n",
            b"data: world\r",
            b"\n",
            b"\r",
            b"\n",
        ];

        let mut events = Vec::new();
        for c in chunks {
            events.extend(p.push(c));
        }

        assert_eq!(
            events,
            vec![SseEvent {
                id: Some("1".to_string()),
                event: Some("greeting".to_string()),
                data: "hello\nworld".to_string(),
                retry: None,
            }]
        );
    }

    #[test]
    fn ignores_non_integer_retry() {
        let mut p = SseParser::new();
        let chunks = vec![b"retry: def\n".as_slice(), b"\n"];
        let mut events = Vec::new();
        for c in chunks {
            events.extend(p.push(c));
        }

        assert_eq!(
            events,
            vec![SseEvent {
                id: None,
                event: None,
                data: "".to_string(),
                retry: None,
            }]
        );
    }

    #[test]
    fn data_appends_across_multiple_lines() {
        let mut p = SseParser::new();
        let chunks = vec![
            b"data:YHOO\n".as_slice(),
            b"data: +2\n",
            b"data\n",
            b"data: 10\n",
            b"\n",
        ];
        let mut events = Vec::new();
        for c in chunks {
            events.extend(p.push(c));
        }

        assert_eq!(
            events[0].data,
            "YHOO\n+2\n\n10".to_string()
        );
    }
}

