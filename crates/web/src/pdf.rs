//! Bridge to pdf.js (vendored in `public/vendor/pdfjs`): reads the OCR text layer
//! of the uploaded PDF and renders page snapshots for image-based questions.

use examcore::layout::RawPage;
use examcore::parse::Region;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r##"
const BASE = "/vendor/pdfjs/";
let pdfjs = null;
let doc = null;
const cache = new Map();

async function lib() {
  if (!pdfjs) {
    pdfjs = await import(BASE + "pdf.min.mjs");
    pdfjs.GlobalWorkerOptions.workerSrc = BASE + "pdf.worker.min.mjs";
  }
  return pdfjs;
}

export async function pdf_open(inputId) {
  const el = document.getElementById(inputId);
  const file = el && el.files && el.files[0];
  if (!file) throw new Error("No file selected");
  const pdf = await lib();
  if (doc) { await doc.destroy(); doc = null; }
  cache.clear();
  const data = new Uint8Array(await file.arrayBuffer());
  doc = await pdf.getDocument({
    data,
    wasmUrl: BASE + "wasm/",
    iccUrl: BASE + "wasm/",
    standardFontDataUrl: BASE + "standard_fonts/",
    isEvalSupported: false,
  }).promise;
  return JSON.stringify({ pages: doc.numPages, name: file.name, size: file.size });
}

export async function pdf_page_text(index) {
  const page = await doc.getPage(index + 1);
  const vp = page.getViewport({ scale: 1 });
  const content = await page.getTextContent();
  const items = [];
  for (const it of content.items) {
    if (!it.str || !it.str.trim()) continue;
    const tx = pdfjs.Util.transform(vp.transform, it.transform);
    const h = Math.hypot(tx[2], tx[3]) || it.height || 1;
    items.push({ s: it.str, x: tx[4], y: tx[5] - h, w: it.width, h });
  }
  page.cleanup();
  return JSON.stringify({ width: vp.width, height: vp.height, items });
}

async function renderPage(index, scale) {
  const key = index + ":" + scale;
  if (cache.has(key)) return cache.get(key);
  const page = await doc.getPage(index + 1);
  const vp = page.getViewport({ scale });
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(vp.width);
  canvas.height = Math.ceil(vp.height);
  const ctx = canvas.getContext("2d");
  await page.render({ canvas, canvasContext: ctx, viewport: vp }).promise;
  page.cleanup();
  cache.set(key, canvas);
  while (cache.size > 3) cache.delete(cache.keys().next().value);
  return canvas;
}

export async function pdf_snapshot(regionsJson, scale, quality) {
  const regions = JSON.parse(regionsJson);
  const parts = [];
  for (const r of regions) {
    const canvas = await renderPage(r.page, scale);
    const sy = Math.max(0, Math.floor(r.top * scale));
    const ey = Math.min(canvas.height, Math.ceil(r.bottom * scale));
    if (ey - sy > 2) parts.push({ canvas, sy, sh: ey - sy });
  }
  if (!parts.length) return null;
  const out = document.createElement("canvas");
  out.width = Math.max(...parts.map((p) => p.canvas.width));
  out.height = parts.reduce((n, p) => n + p.sh, 0);
  const ctx = out.getContext("2d");
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, out.width, out.height);
  let y = 0;
  for (const p of parts) {
    ctx.drawImage(p.canvas, 0, p.sy, p.canvas.width, p.sh, 0, y, p.canvas.width, p.sh);
    y += p.sh;
  }
  const blob = await new Promise((res) => out.toBlob(res, "image/webp", quality));
  return [new Uint8Array(await blob.arrayBuffer()), blob.type];
}

export async function pdf_close() {
  if (doc) await doc.destroy();
  doc = null;
  cache.clear();
}
"##)]
extern "C" {
    #[wasm_bindgen(catch, js_name = pdf_open)]
    async fn js_open(input_id: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = pdf_page_text)]
    async fn js_page_text(index: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = pdf_snapshot)]
    async fn js_snapshot(regions: &str, scale: f64, quality: f64) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = pdf_close)]
    async fn js_close() -> Result<JsValue, JsValue>;
}

fn js_err(e: JsValue) -> String {
    e.dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct PdfInfo {
    pub pages: u32,
    pub name: String,
    pub size: f64,
}

/// Opens the file selected in `<input type=file id={input_id}>`.
pub async fn open(input_id: &str) -> Result<PdfInfo, String> {
    let v = js_open(input_id).await.map_err(js_err)?;
    serde_json::from_str(&v.as_string().unwrap_or_default()).map_err(|e| e.to_string())
}

pub async fn page_text(index: u32) -> Result<RawPage, String> {
    let v = js_page_text(index).await.map_err(js_err)?;
    serde_json::from_str(&v.as_string().unwrap_or_default()).map_err(|e| e.to_string())
}

pub struct Snapshot {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

impl Snapshot {
    pub fn extension(&self) -> &'static str {
        match self.content_type.as_str() {
            "image/webp" => "webp",
            "image/jpeg" => "jpg",
            _ => "png",
        }
    }
}

/// Renders the regions (stacked vertically) into one image.
pub async fn snapshot(regions: &[Region], scale: f64) -> Result<Option<Snapshot>, String> {
    if regions.is_empty() {
        return Ok(None);
    }
    let json = serde_json::to_string(regions).map_err(|e| e.to_string())?;
    let v = js_snapshot(&json, scale, 0.82).await.map_err(js_err)?;
    if v.is_null() || v.is_undefined() {
        return Ok(None);
    }
    let arr = js_sys::Array::from(&v);
    let bytes = js_sys::Uint8Array::new(&arr.get(0)).to_vec();
    let content_type = arr.get(1).as_string().unwrap_or_else(|| "image/png".into());
    Ok(Some(Snapshot {
        bytes,
        content_type,
    }))
}

pub async fn close() {
    let _ = js_close().await;
}
