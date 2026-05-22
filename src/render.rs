//! Render an HTML scene to RGBA via upstream Blitz.
//!
//! Two backends are compiled in: GPU (anyrender_vello → wgpu) is the
//! default; CPU (anyrender_vello_cpu) is the escape hatch for headless
//! environments without a GPU adapter, or when the agent / operator forces
//! it via `wavelet --cpu`.
//!
//! Backend selection is a process-wide latch (`BACKEND`) initialized on
//! first render. Once set, every subsequent render uses the same path —
//! no mid-stream switching, no per-frame probing.

use anyrender::{ImageRenderer, PaintScene as _};
use anyrender_vello::VelloImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use kurbo::Rect;
use peniko::{Color, Fill};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

/// Render backend in effect for the current process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    /// GPU Vello via wgpu. Default when a compatible adapter is present.
    Gpu,
    /// CPU Vello. Used when GPU probe fails or `force_cpu` was called.
    Cpu,
}

const BACKEND_UNSET: u8 = 0;
const BACKEND_GPU: u8 = 1;
const BACKEND_CPU: u8 = 2;

/// Process-wide latch. Set on first render or via [`force_cpu`].
static BACKEND: AtomicU8 = AtomicU8::new(BACKEND_UNSET);

/// Force the CPU backend for the rest of this process. Call this before any
/// `render_document_to_rgba`. Idempotent; later calls are a no-op.
pub fn force_cpu() {
    BACKEND
        .compare_exchange(BACKEND_UNSET, BACKEND_CPU, Ordering::SeqCst, Ordering::SeqCst)
        .ok();
}


/// Probe wgpu for a usable adapter. Returns true iff `Instance::request_adapter`
/// resolves to Some. Probe result is cached for the process lifetime — the
/// probe takes ~100ms cold so we don't repeat it.
fn gpu_available() -> bool {
    static PROBE: OnceLock<bool> = OnceLock::new();
    *PROBE.get_or_init(|| {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }));
        adapter.is_ok()
    })
}

/// Resolves `file://` URLs by reading from the local filesystem. Synchronous
/// (calls `handler.bytes` inline) so image / stylesheet loads complete in time
/// for the document's first `resolve(0.0)` call. Non-file schemes return
/// silently — no network access in the offline render pipeline.
pub struct FileNetProvider;

impl NetProvider for FileNetProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url;
        if url.scheme() != "file" {
            return;
        }
        let path: PathBuf = match url.to_file_path() {
            Ok(p) => p,
            Err(_) => return,
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        handler.bytes(url.as_str().to_owned(), Bytes::from(bytes));
    }
}

/// Long-lived renderer holding the wgpu device (GPU) or the CPU rasterizer
/// state. The GPU init alone is ~100ms; reusing it across frames is the
/// difference between GPU being faster and slower than CPU on offline render.
///
/// Typical use:
/// ```ignore
/// let mut r = Renderer::new(1280, 720);
/// for frame in 0..total {
///     let rgba = r.render(&mut doc);
///     encoder.push_frame(&rgba)?;
/// }
/// ```
pub struct Renderer {
    width: u32,
    height: u32,
    backend: RenderBackend,
    gpu: Option<VelloImageRenderer>,
    cpu: Option<VelloCpuImageRenderer>,
    buf: Vec<u8>,
}

impl Renderer {
    /// Construct a renderer at `(width, height)`. Probes the active backend
    /// on first use and reuses it; subsequent renderers in the same process
    /// pick the same backend.
    pub fn new(width: u32, height: u32) -> Self {
        let backend = pick_backend();
        // One-line diagnostic so eval / debug runs can see which backend
        // was selected without grepping wgpu's own logs. Printed once per
        // Renderer construction; cheap and load-bearing for triaging
        // filter / paint perf reports.
        eprintln!(
            "wavelet render: backend={:?} ({}×{})",
            backend, width, height
        );
        let (gpu, cpu) = match backend {
            RenderBackend::Gpu => (Some(VelloImageRenderer::new(width, height)), None),
            RenderBackend::Cpu => (None, Some(VelloCpuImageRenderer::new(width, height))),
        };
        Self {
            width,
            height,
            backend,
            gpu,
            cpu,
            buf: Vec::with_capacity((width * height * 4) as usize),
        }
    }

    /// The dimensions the renderer is configured for.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The active backend.
    pub fn backend(&self) -> RenderBackend {
        self.backend
    }

    /// Paint the resolved document into RGBA with a transparent background
    /// (alpha = 0 where no element draws). Use this when compositing the
    /// HTML output over another image / video frame downstream.
    pub fn render_transparent(&mut self, doc: &mut BaseDocument) -> Vec<u8> {
        self.render_inner(doc, false)
    }

    /// Paint the resolved document into RGBA. Reuses the underlying GPU /
    /// CPU renderer between calls — only the scene state is reset.
    pub fn render(&mut self, doc: &mut BaseDocument) -> Vec<u8> {
        self.render_inner(doc, true)
    }

    fn render_inner(&mut self, doc: &mut BaseDocument, opaque_white_bg: bool) -> Vec<u8> {
        let w = self.width;
        let h = self.height;
        let bg = Rect::new(0.0, 0.0, w as f64, h as f64);
        self.buf.clear();
        match self.backend {
            RenderBackend::Gpu => {
                let r = self.gpu.as_mut().expect("GPU renderer not constructed");
                r.reset();
                r.render_to_vec(
                    |scene| {
                        if opaque_white_bg {
                            scene.fill(Fill::NonZero, Default::default(), Color::WHITE, Default::default(), &bg);
                        }
                        paint_scene(scene, doc, 1.0, w, h, 0, 0);
                    },
                    &mut self.buf,
                );
            }
            RenderBackend::Cpu => {
                let r = self.cpu.as_mut().expect("CPU renderer not constructed");
                r.reset();
                r.render_to_vec(
                    |scene| {
                        if opaque_white_bg {
                            scene.fill(Fill::NonZero, Default::default(), Color::WHITE, Default::default(), &bg);
                        }
                        paint_scene(scene, doc, 1.0, w, h, 0, 0);
                    },
                    &mut self.buf,
                );
            }
        }
        // Return a fresh Vec — keeps the caller's contract simple. The
        // internal buffer is preserved for the next reuse.
        self.buf.clone()
    }
}

/// One-shot convenience that allocates a fresh `Renderer` per call. **Avoid
/// in frame loops** — see `Renderer::new` + `Renderer::render` for the
/// per-frame-amortized path.
pub fn render_document_to_rgba(doc: &mut BaseDocument, width: u32, height: u32) -> Vec<u8> {
    let mut r = Renderer::new(width, height);
    r.render(doc)
}

/// Resolve the active backend, probing GPU once on first use.
fn pick_backend() -> RenderBackend {
    match BACKEND.load(Ordering::SeqCst) {
        BACKEND_GPU => return RenderBackend::Gpu,
        BACKEND_CPU => return RenderBackend::Cpu,
        _ => {}
    }
    let chosen = if gpu_available() {
        BACKEND_GPU
    } else {
        eprintln!("wavelet: no GPU adapter found — falling back to CPU Vello.");
        BACKEND_CPU
    };
    // Race-safe: first writer wins, subsequent renders see the same backend.
    BACKEND
        .compare_exchange(BACKEND_UNSET, chosen, Ordering::SeqCst, Ordering::SeqCst)
        .ok();
    if chosen == BACKEND_GPU {
        RenderBackend::Gpu
    } else {
        RenderBackend::Cpu
    }
}

/// Build a parsed + resolved `HtmlDocument` from an HTML string. `base_url`,
/// when supplied, is used to resolve relative URLs in the document (e.g.
/// `<img src="../assets/foo.jpg">`). For scenes loaded off disk, pass the
/// scene file's `file://` URL so relative asset paths resolve.
///
/// Call `document.as_mut().resolve(0.0)` after any subsequent mutations.
pub fn load_html(html: &str, width: u32, height: u32) -> HtmlDocument {
    load_html_with_base(html, width, height, None)
}

/// Like [`load_html`] but also wires a `file://` net provider + a base URL
/// so relative `<img src>` / `<link href>` references resolve against
/// `base_url`.
pub fn load_html_with_base(
    html: &str,
    width: u32,
    height: u32,
    base_url: Option<String>,
) -> HtmlDocument {
    let mut document = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url,
            viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
            net_provider: Some(Arc::new(FileNetProvider) as Arc<dyn NetProvider>),
            ..Default::default()
        },
    );
    // FileNetProvider delivers bytes synchronously via the document's mpsc
    // tx; `handle_messages` drains those into the document so images +
    // stylesheets register before layout. Without this `<img>` elements
    // never get their pixels, and `<link rel=stylesheet>` never applies.
    // Loop until quiescent — each handle_messages pass may queue further
    // fetches (e.g. a CSS file referencing fonts).
    for _ in 0..4 {
        document.as_mut().handle_messages();
        document.as_mut().resolve(0.0);
    }
    document
}

/// One-shot helper: render an HTML string to RGBA, no animation.
pub fn render_html_to_rgba(html: &str, width: u32, height: u32) -> Vec<u8> {
    let mut document = load_html(html, width, height);
    render_document_to_rgba(document.as_mut(), width, height)
}
