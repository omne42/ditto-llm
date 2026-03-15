use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ditto_core::contracts::{ContentPart, FileSource, ImageSource, Message, Role};
use ditto_core::error::{DittoError, Result};
use ditto_core::llm_core::model::LanguageModel;
use ditto_core::providers::OpenAI;

fn guess_image_media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/jpeg",
    }
}

fn read_base64(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|err| {
        DittoError::invalid_response_text(format!("read {}: {err}", path.display()))
    })?;
    Ok(STANDARD.encode(bytes))
}

#[tokio::main]
async fn main() -> Result<()> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| DittoError::invalid_response_text("missing OPENAI_API_KEY".to_string()))?;
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let mut args = std::env::args().skip(1);
    let image_path = args.next().map(PathBuf::from);
    let pdf_path = args.next().map(PathBuf::from);

    if image_path.is_none() && pdf_path.is_none() {
        return Err(DittoError::invalid_response_text(
            "usage: cargo run --example multimodal -- <image_path?> <pdf_path?>".to_string(),
        ));
    }

    let mut parts = vec![ContentPart::Text {
        text: "Describe what you see and summarize any attached document.".to_string(),
    }];

    if let Some(path) = image_path {
        let media_type = guess_image_media_type(&path).to_string();
        parts.push(ContentPart::Image {
            source: ImageSource::Base64 {
                media_type,
                data: read_base64(&path)?,
            },
        });
    }

    if let Some(path) = pdf_path {
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        parts.push(ContentPart::File {
            filename,
            media_type: "application/pdf".to_string(),
            source: FileSource::Base64 {
                data: read_base64(&path)?,
            },
        });
    }

    let openai = OpenAI::new(api_key).with_model(model);
    let messages = vec![Message {
        role: Role::User,
        content: parts,
    }];

    let response = openai.generate(messages.into()).await?;
    println!("{}", response.text());
    Ok(())
}
