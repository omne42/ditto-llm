# Images

Ditto 通过 `ImageGenerationModel` trait 提供统一的图片生成接口（对齐 OpenAI `/images/generations` 的核心形状）。

## 请求/响应类型

- `ImageGenerationRequest`：`prompt` + 可选 `model`/`n`/`size`/`response_format`
- `ImageGenerationResponse`：`images: Vec<ImageSource>`（URL 或 Base64）

## 最小示例

```rust
use ditto_llm::{ImageGenerationRequest, ImageResponseFormat, OpenAIImages};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let images = OpenAIImages::new(std::env::var("OPENAI_API_KEY")?)
        .with_model("gpt-image-1");

    let resp = images
        .generate(ImageGenerationRequest {
            prompt: "A minimal flat icon of a rust crab.".to_string(),
            model: None,
            n: Some(1),
            size: Some("1024x1024".to_string()),
            response_format: Some(ImageResponseFormat::Url),
            provider_options: None,
        })
        .await?;

    println!("images={:?}", resp.images);
    println!("warnings={:?}", resp.warnings);
    Ok(())
}
```

> 注意：`OpenAIImages` 需要开启 features `openai` + `images`；OpenAI-compatible 也有对应的 images client（见 crate re-exports）。

## 常见坑

- **返回格式**：`response_format` 选择 `Url` 或 `Base64Json`；不同 provider/网关可能只支持其一。
- **size 的约束**：size 的合法值通常由 provider 决定，Ditto 仅做 best-effort 透传。
- **成本与带宽**：Base64 会显著增大 payload；生产建议优先使用 URL 或把图片落到对象存储。
