# rust-sse

这是一个“边 vibe 边学习”的 Rust 工程：先实现 **SSE（`text/event-stream`）解析器**，再在其上实现 **可流式消费的 HTTP SSE 客户端**。

## 目标（方向 A）

- 纯解析器：输入任意切分的字节 chunks（模拟网络分片），输出结构化事件
- 关键边界：`\n`/`\r`/`\r\n`、`\r\n` 跨 chunk、多行 `data:` 拼接、comment 行、`retry` 非整数忽略、`id:` 为空清空 Last-Event-ID
- 单元测试：覆盖典型坑点，并能“观察”每个阶段的输入输出

## 当前进度

- 已完成：解析器（chunk -> line -> message）与单测
- 待完成：网络层（`reqwest`/`tokio`）+ 重连策略 + `Last-Event-ID`

