# Warnings 与兼容性

Ditto 的一个核心原则是：**差异要可见**。

当某个 provider 不支持某个字段/能力，或者 Ditto 为了保证稳定性对输入做了处理（例如 clamp 温度/丢弃非有限值），Ditto 会尽量：

- 把字段做 best-effort 映射/降级
- 同时通过 `Warning` 把“发生了什么”告诉调用方

这样做的好处：

- 你可以在日志/观测里定位“为什么某个模型没按预期工作”
- 你可以在 Gateway 侧按 Warning 做策略（例如拒绝不兼容请求）

在 SDK 示例里你会频繁看到 `response.warnings`，建议在生产环境把它打到日志（并注意脱敏）。
