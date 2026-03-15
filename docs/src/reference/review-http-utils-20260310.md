# 代码审查：`crates/ditto-core/src/provider_transport/http.rs`（2026-03-10）

## 范围

- 目标文件：`crates/ditto-core/src/provider_transport/http.rs`
- 审查重点：correctness、资源使用、helper API 设计

## 主要问题

### 1. bounded-body helper 的 API 设计错误

`read_reqwest_body_bytes_bounded_with_content_length` 要求调用方额外传 `headers: &HeaderMap`，但 `reqwest::Response` 本身已经有 `content_length()`。

结果是多个调用点被迫先 `response.headers().clone()`，再把 `response` move 进去：

- `crates/ditto-core/src/providers/openai_audio_common.rs`
- `crates/ditto-core/src/providers/openai_like.rs`
- `crates/ditto-core/src/providers/openai_videos_common.rs`

这不是调用方的问题，是 helper 签名把调用方逼成了低质量代码。

### 2. `send_checked_bytes` 会先下载，再在成功路径报超限

对未知 `content-length` 的成功响应，`send_checked_bytes` 会先把 body 读进内存，再根据 `truncated` 返回错误。

这意味着：

- 先消耗带宽
- 先分配内存
- 最后把结果丢掉

对 bounded body 场景，更合理的行为是读取过程中一旦超限就立即失败。

### 3. 同类响应体上限逻辑重复实现且开始漂移

当前至少有三份相似实现：

- `response_bytes_truncated`
- `read_reqwest_body_bytes_bounded_with_content_length`
- `crates/ditto-server/src/gateway/transport/http/proxy_bounded_body.rs`

继续保留这三套逻辑，只会让不同调用方拥有不同的超限语义。

### 4. `String::from_utf8_lossy(...).to_string()` 存在不必要分配

在错误体转字符串时，当前实现直接把 `Cow<str>` `.to_string()`。这里更合理的是 `.into_owned()`。

这不是最大问题，但属于明显可以顺手修掉的低质量标准库用法。

## 建议的重构顺序

1. 合并 bounded-body 读取逻辑，收敛成一套基础实现
2. 去掉 `headers: &HeaderMap` 参数，消灭调用方的 `headers().clone()`
3. 明确区分“严格超限立即失败”和“允许截断只用于错误展示”两种模式
4. 顺手把 `to_string()` 改成 `into_owned()`

## 结论

值得重构。

核心原因不是代码风格，而是：

- helper API 正在污染调用方
- 资源使用策略不一致
- 边界逻辑已经分叉成多份实现
