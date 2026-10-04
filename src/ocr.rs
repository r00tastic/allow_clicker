use std::io::Cursor;

use anyhow::{anyhow, Result};
use image::DynamicImage;
use windows::{
    Graphics::Imaging::{BitmapDecoder, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
};

#[derive(Debug, Clone)]
pub struct Hit {
    pub text: String,
    pub center_x: i32,
    pub center_y: i32,
}

pub struct Ocr {
    engine: OcrEngine,
}

impl Ocr {
    pub fn new() -> Result<Self> {
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .map_err(|e| anyhow!("OCR engine: {}", e))?;
        Ok(Self { engine })
    }

    /// Herhangi bir hedef ifadeyi ara (tek kelime veya cok kelimeli).
    /// Her hedef whitespace ile bolunur, OCR line'inda ardisik kelimelere sliding-window
    /// match uygulanir. Bulunan ilk match'in birlesik bounding merkezi doner.
    pub fn find_any(
        &self,
        img: &DynamicImage,
        targets: &[String],
        case_sensitive: bool,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<Option<Hit>> {
        if targets.is_empty() {
            return Ok(None);
        }
        let bitmap = dyn_to_softwarebitmap(img)?;
        let result = self.engine.RecognizeAsync(&bitmap)?.get()?;

        // Her target'i whitespace ile bol
        let target_sets: Vec<(String, Vec<String>)> = targets
            .iter()
            .filter_map(|t| {
                let parts: Vec<String> = t
                    .split_whitespace()
                    .map(|w| normalize(w, case_sensitive))
                    .collect();
                if parts.is_empty() {
                    None
                } else {
                    Some((t.clone(), parts))
                }
            })
            .collect();

        if target_sets.is_empty() {
            return Ok(None);
        }

        let lines = result.Lines()?;
        for line in &lines {
            let words = line.Words()?;
            // Line'daki (normalized_text, bounding_rect) tuple listesi
            let mut line_words: Vec<(String, windows::Foundation::Rect)> = Vec::new();
            for w in &words {
                let raw = w.Text().map(|s| s.to_string()).unwrap_or_default();
                let norm = normalize(&raw, case_sensitive);
                let rect = w.BoundingRect()?;
                line_words.push((norm, rect));
            }

            // Her hedef seti icin sliding window
            for (orig, tset) in &target_sets {
                let n = tset.len();
                if line_words.len() < n {
                    continue;
                }
                for i in 0..=(line_words.len() - n) {
                    let mut ok = true;
                    for j in 0..n {
                        if line_words[i + j].0 != tset[j] {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        // Birlesik bounding rect
                        let first = &line_words[i].1;
                        let last = &line_words[i + n - 1].1;
                        let left = first.X;
                        let top = first.Y;
                        let right = last.X + last.Width;
                        let bottom = first.Y + first.Height;
                        let cx = offset_x + left as i32 + ((right - left) as i32) / 2;
                        let cy = offset_y + top as i32 + ((bottom - top) as i32) / 2;
                        return Ok(Some(Hit {
                            text: orig.clone(),
                            center_x: cx,
                            center_y: cy,
                        }));
                    }
                }
            }
        }
        Ok(None)
    }
}

/// Kelime normalizasyonu: case-insensitive ise lowercase, ayrica basit noktalama sil.
fn normalize(s: &str, case_sensitive: bool) -> String {
    let stripped: String = s
        .chars()
        .filter(|c| !matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | '"' | '\'' | '(' | ')' | '[' | ']'))
        .collect();
    if case_sensitive {
        stripped
    } else {
        stripped.to_lowercase()
    }
}

fn dyn_to_softwarebitmap(img: &DynamicImage) -> Result<SoftwareBitmap> {
    let mut buf: Vec<u8> = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| anyhow!("PNG encode: {}", e))?;

    let stream = InMemoryRandomAccessStream::new()?;
    let out = stream.GetOutputStreamAt(0)?;
    let writer = DataWriter::CreateDataWriter(&out)?;
    writer.WriteBytes(&buf)?;
    writer.StoreAsync()?.get()?;
    writer.FlushAsync()?.get()?;
    writer.DetachStream()?;

    let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
    let bitmap = decoder.GetSoftwareBitmapAsync()?.get()?;
    Ok(bitmap)
}
