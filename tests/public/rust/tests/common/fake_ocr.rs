//! Strict OCR request contract for the loopback Mistral fixture. Uses real file parsers;
//! fake_mistral calls this before supplying scripted text. Accepted formats follow what
//! the real OCR service accepted in a live documents run; Office files are read locally, never OCR.
use base64::Engine as _;
use serde_json::Value;

pub const MAX_FILE: usize = 50 * 1024 * 1024;

pub fn validate(body: &Value) -> Result<(String, usize, usize), &'static str> {
    if body["model"] != "mistral-ocr-latest" {
        return Err("Invalid OCR model.");
    }
    let doc = &body["document"];
    let field = match doc["type"].as_str() {
        Some("document_url") => "document_url",
        Some("image_url") => "image_url",
        _ => return Err("Invalid document type."),
    };
    let (mime, data) = doc[field]
        .as_str()
        .and_then(|url| url.strip_prefix("data:"))
        .and_then(|url| url.split_once(";base64,"))
        .ok_or("Invalid inline document URL.")?;
    if data.len() > MAX_FILE.div_ceil(3) * 4 {
        return Err("Document exceeds the 50 MB limit.");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "Invalid base64 document.")?;
    if bytes.is_empty() || bytes.len() > MAX_FILE {
        return Err("Document is empty or exceeds the 50 MB limit.");
    }
    let pages = if field == "document_url" {
        if mime != "application/pdf" {
            return Err("Invalid document type. Only PDF documents are supported.");
        }
        let pdf = lopdf::Document::load_mem(&bytes).map_err(|_| "Invalid or damaged PDF.")?;
        if pdf.is_encrypted() {
            return Err("Protected PDF cannot be read.");
        }
        let pages = pdf.get_pages().len();
        if pages == 0 || pages > 1000 {
            return Err("PDF must contain between 1 and 1000 pages.");
        }
        pages
    } else {
        let format = match mime {
            "image/png" => Some(image::ImageFormat::Png),
            "image/jpeg" => Some(image::ImageFormat::Jpeg),
            "image/webp" => Some(image::ImageFormat::WebP),
            "image/gif" => Some(image::ImageFormat::Gif),
            "image/tiff" => Some(image::ImageFormat::Tiff),
            "image/bmp" => Some(image::ImageFormat::Bmp),
            "image/avif" => None,
            _ => return Err("Unsupported image MIME type."),
        };
        if let Some(format) = format {
            image::load_from_memory_with_format(&bytes, format)
                .map_err(|_| "Invalid image data.")?;
        } else {
            validate_avif(&bytes)?;
        }
        // OCR reads the first image, including the TIFF behavior seen in that live run.
        1
    };
    Ok((mime.to_owned(), bytes.len(), pages))
}

fn validate_avif(bytes: &[u8]) -> Result<(), &'static str> {
    oxideav_avif::AvifDecoder::new(oxideav_core::CodecId::new("avif"))
        .decode_file(bytes)
        .map(|_| ())
        .map_err(|_| "Invalid AVIF image data.")
}
