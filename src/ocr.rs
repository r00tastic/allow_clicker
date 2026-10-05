use anyhow::{anyhow, Result};
use windows::{
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
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

    pub fn find_any(
        &self,
        bgra: &[u8],
        width: u32,
        height: u32,
        targets: &[String],
        case_sensitive: bool,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<Option<Hit>> {
        if targets.is_empty() {
            return Ok(None);
        }
        let bitmap = raw_to_softwarebitmap(bgra, width, height)?;
        let result = self.engine.RecognizeAsync(&bitmap)?.get()?;

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
            let mut line_words: Vec<(String, windows::Foundation::Rect)> = Vec::new();
            for w in &words {
                let raw = w.Text().map(|s| s.to_string()).unwrap_or_default();
                let norm = normalize(&raw, case_sensitive);
                let rect = w.BoundingRect()?;
                line_words.push((norm, rect));
            }

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

fn raw_to_softwarebitmap(bgra: &[u8], w: u32, h: u32) -> Result<SoftwareBitmap> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(bgra)?;
    let ibuffer = writer.DetachBuffer()?;
    SoftwareBitmap::CreateCopyFromBuffer(
        &ibuffer,
        BitmapPixelFormat::Bgra8,
        w as i32,
        h as i32,
    )
    .map_err(|e| anyhow!("SoftwareBitmap: {}", e))
}
