//! SSE 事件类型

/// 一条 SSE 事件（由 field/value 行组成，以空行结束）
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

impl SseEvent {
    /// 使用给定的数据创建新的 SSE 事件
    pub fn new(data: impl Into<String>) -> Self {
        Self {
            data: data.into(),
            ..Default::default()
        }
    }

    /// 设置事件类型
    pub fn with_event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }

    /// 设置事件 ID
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// 设置重试间隔（毫秒）
    pub fn with_retry(mut self, retry_ms: u64) -> Self {
        self.retry = Some(retry_ms);
        self
    }

    /// 将事件格式化为 SSE 文本（用于输出到客户端）
    pub fn to_sse_string(&self) -> String {
        let mut output = String::new();

        if let Some(ref id) = self.id {
            output.push_str("id: ");
            output.push_str(id);
            output.push('\n');
        }

        if let Some(ref event) = self.event {
            output.push_str("event: ");
            output.push_str(event);
            output.push('\n');
        }

        if let Some(retry) = self.retry {
            output.push_str("retry: ");
            output.push_str(&retry.to_string());
            output.push('\n');
        }

        // 处理多行数据
        for line in self.data.lines() {
            output.push_str("data: ");
            output.push_str(line);
            output.push('\n');
        }

        // 空数据也需要 data 字段
        if self.data.is_empty() {
            output.push_str("data:\n");
        }

        output.push('\n'); // 空行结束事件
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_sse_string_simple() {
        let event = SseEvent::new("hello");
        assert_eq!(event.to_sse_string(), "data: hello\n\n");
    }

    #[test]
    fn test_to_sse_string_with_all_fields() {
        let event = SseEvent::new("hello")
            .with_id("123")
            .with_event("message")
            .with_retry(5000);

        let output = event.to_sse_string();
        assert!(output.contains("id: 123\n"));
        assert!(output.contains("event: message\n"));
        assert!(output.contains("retry: 5000\n"));
        assert!(output.contains("data: hello\n"));
        assert!(output.ends_with("\n\n"));
    }

    #[test]
    fn test_to_sse_string_multiline() {
        let event = SseEvent::new("line1\nline2\nline3");
        let output = event.to_sse_string();
        assert_eq!(output, "data: line1\ndata: line2\ndata: line3\n\n");
    }
}
