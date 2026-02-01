# Audio（transcriptions / speech）

Ditto 提供两类音频能力（feature `audio`）：

- **Transcriptions / Translations**：语音转文字（对齐 OpenAI `/audio/transcriptions` 与 `/audio/translations`）
- **Speech**：文字转语音（对齐 OpenAI `/audio/speech`）

## 需要的 features

- OpenAI：`openai` + `audio`
- OpenAI-compatible：`openai-compatible` + `audio`

对应的 client（crate re-exports）：

- `OpenAIAudioTranscription` / `OpenAISpeech`
- `OpenAICompatibleAudioTranscription` / `OpenAICompatibleSpeech`

## Transcriptions：最小示例

```rust
use ditto_llm::{AudioTranscriptionRequest, OpenAIAudioTranscription, TranscriptionResponseFormat};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into())
    })?;
    let model = OpenAIAudioTranscription::new(api_key).with_model("whisper-1");

    let resp = model
        .transcribe(AudioTranscriptionRequest {
            audio: std::fs::read("audio.wav")?,
            filename: "audio.wav".to_string(),
            media_type: Some("audio/wav".to_string()),
            model: None, // 也可以在这里覆盖
            language: None,
            prompt: None,
            response_format: Some(TranscriptionResponseFormat::Json),
            temperature: None,
            provider_options: None,
        })
        .await?;

    println!("{}", resp.text);
    Ok(())
}
```

## Speech：最小示例

```rust
use ditto_llm::{OpenAISpeech, SpeechRequest, SpeechResponseFormat};

#[tokio::main]
async fn main() -> ditto_llm::Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ditto_llm::DittoError::InvalidResponse("missing OPENAI_API_KEY".into())
    })?;
    let tts = OpenAISpeech::new(api_key).with_model("gpt-4o-mini-tts");

    let resp = tts
        .speak(SpeechRequest {
            input: "Hello from Ditto.".to_string(),
            voice: "alloy".to_string(),
            model: None,
            response_format: Some(SpeechResponseFormat::Mp3),
            speed: Some(1.0),
            provider_options: None,
        })
        .await?;

    std::fs::write("out.mp3", resp.audio)?;
    Ok(())
}
```

## 内存与性能注意事项

当前音频 API 以 `Vec<u8>` 形式读入/返回音频内容：

- 请求：`AudioTranscriptionRequest.audio: Vec<u8>`
- 响应：`SpeechResponse.audio: Vec<u8>`

这意味着大文件会带来较高的内存峰值。若你在网关/高并发服务中使用，建议：

- 对上传大小做限制（例如反向代理层或业务层）
- 将音频落盘或对象存储，再传 URL（若 provider 支持）
- 避免在日志里记录音频内容，务必脱敏
