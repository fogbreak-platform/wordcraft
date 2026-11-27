#[cfg(target_arch = "wasm32")]
mod host_bridge;
#[cfg(target_arch = "wasm32")]
mod theme;

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

#[cfg(target_arch = "wasm32")]
fn main() {
    wasm_main();
}

#[cfg(target_arch = "wasm32")]
fn wasm_main() {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    use craft::automation::control::{ControlChannel, ControlRequest};
    use craft::host::{FileError, Files, Host, PendingPick, PickId, PickRequest, Picked, Wake, WriteIntent};
    use craft::platform::appearance::SystemReader;
    use craft::platform::browser_files::BrowserFiles;
    use craft::platform::clipboard::{Clipboard, ClipboardError, EventClipboard, Flavor, LateOffers, TEXT};
    use craft::platform::open::{FileIntake, IntakeSender};
    use craft::print::spool::Spool;
    use craft::settings::{LocalStorageFiles, Location, OpenOptions, SettingsStore, SharedSettings};
    use craft::ui::InstallSpec;
    use craft::ui::shell::embed;
    use craft::window::system::NoWindows;
    use serde_json::json;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{ClipboardEvent, HtmlCanvasElement};
    use wordcraft_engine::Session;
    use wordcraft_ui_egui::prefs::Prefs;
    use wordcraft_ui_egui::{Services, WordApp};

    use crate::host_bridge::{ActionQueue, ExportResult, HostAction, HostContext};

    const MAX_PASTED_BYTES: f64 = (200u32 << 20) as f64;
    const MAX_PASTED_PICTURES: u32 = 20;
    const MAX_STORED_PREFS: usize = 4 << 20;
    const LOADING_ID: &str = "fogbreak_loading";

    struct OverlayFiles {
        inner: Arc<BrowserFiles>,
        canvas: HtmlCanvasElement,
        answers: Mutex<HashMap<u64, Picked>>,
    }

    impl OverlayFiles {
        fn new(inner: Arc<BrowserFiles>, canvas: HtmlCanvasElement) -> Self {
            Self {
                inner,
                canvas,
                answers: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Files for OverlayFiles {
        fn pick(&self, request: PickRequest) -> PickId {
            self.inner.pick(request)
        }

        fn take_answer(&self, id: PickId) -> Option<Picked> {
            if let Some(ans) = self.answers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id.0) {
                return Some(ans);
            }
            self.inner.take_answer(id)
        }

        fn forget(&self, id: PickId) {
            if self.answers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id.0).is_none() {
                self.inner.forget(id);
            }
        }

        fn pending_pick(&self) -> Option<PendingPick> {
            self.inner.pending_pick()
        }

        fn read(&self, path: &Path) -> Result<Vec<u8>, FileError> {
            self.inner.read(path)
        }

        fn write(&self, path: &Path, bytes: &[u8], intent: WriteIntent) -> Result<bool, FileError> {
            self.inner.write(path, bytes, intent)
        }

        fn last_folder(&self, purpose: &str) -> Option<PathBuf> {
            self.inner.last_folder(purpose)
        }

        fn save(&self, request: PickRequest, bytes: Vec<u8>) -> PickId {
            let name = request.file_name.unwrap_or_else(|| "Document.docx".to_string());
            host_bridge::deliver_save(&self.canvas, &name, request.save_as, &bytes);
            let answer = if request.save_as {
                Picked::Saved {
                    path: PathBuf::from(&name),
                    file_type: None,
                }
            } else {
                Picked::Downloaded(name)
            };
            let id = self.inner.pick(PickRequest::folder("overlay-save"));
            self.inner.forget(id);
            self.answers.lock().unwrap_or_else(|e| e.into_inner()).insert(id.0, answer);
            id
        }

        fn can_pick(&self) -> bool {
            self.inner.can_pick()
        }

        fn browser_saves(&self) -> bool {
            true
        }
    }

    struct WebShell {
        app: WordApp,
        sender: IntakeSender,
        settings: SharedSettings<Prefs>,
        actions: ActionQueue,
        canvas: HtmlCanvasElement,
        dark_state: Arc<Mutex<bool>>,
    }

    fn query_string() -> String {
        web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
    }

    fn waker(ctx: &egui::Context) -> impl Fn() + Send + Sync + Clone + 'static {
        let ctx = ctx.clone();
        move || ctx.request_repaint()
    }

    fn copy_out(ctx: &egui::Context, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        let (index, text) = flavors
            .iter()
            .enumerate()
            .find(|(_, f)| f.mime == TEXT)
            .ok_or(ClipboardError::NoFormat)?;
        ctx.copy_text(String::from_utf8_lossy(&text.data).into_owned());
        Ok(index)
    }

    fn listen_for_pasted_pictures(late: LateOffers) -> Result<(), String> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let late = Rc::new(late);
        let on_paste = Closure::<dyn FnMut(ClipboardEvent)>::new(move |e: ClipboardEvent| {
            let Some(data) = e.clipboard_data() else { return };
            if !data.get_data("text").unwrap_or_default().is_empty() {
                return;
            }
            let Some(files) = data.files() else { return };
            for i in 0..files.length().min(MAX_PASTED_PICTURES) {
                let Some(file) = files.get(i) else { continue };
                if !file.type_().starts_with("image/") || file.size() > MAX_PASTED_BYTES {
                    continue;
                }
                let late = late.clone();
                let mime = file.type_();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        late.offer(vec![Flavor::new(mime, bytes)]);
                    }
                });
            }
        });
        document
            .add_event_listener_with_callback("paste", on_paste.as_ref().unchecked_ref())
            .map_err(|e| format!("{e:?}"))?;
        on_paste.forget();
        Ok(())
    }

    fn export_mime(format: &str) -> &'static str {
        match format {
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "docm" => "application/vnd.ms-word.document.macroEnabled.12",
            "dotx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
            "dotm" => "application/vnd.ms-word.template.macroEnabled.12",
            "pdf" => "application/pdf",
            "odt" => "application/vnd.oasis.opendocument.text",
            "ott" => "application/vnd.oasis.opendocument.text-template",
            "html" | "htm" => "text/html",
            "rtf" => "application/rtf",
            "md" => "text/markdown",
            "txt" => "text/plain",
            "xml" => "application/xml",
            "png" => "image/png",
            "json" => "application/json",
            _ => "application/octet-stream",
        }
    }

    fn normalize_export_name(name: Option<String>, format: &str, title: &str) -> String {
        let stem = title.trim();
        let fallback_stem = if stem.is_empty() { "Document" } else { stem };
        let base = name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("{fallback_stem}.{format}"));
        if base.ends_with(&format!(".{format}")) {
            base
        } else {
            format!("{base}.{format}")
        }
    }

    fn perform_export(app: &WordApp, format: &str, name: Option<String>, page: u64, scale: f32) -> Result<ExportResult, String> {
        let doc_name = normalize_export_name(name, format, &app.title_stem());
        let opts = wordcraft_engine::io::ExportOptions::default();
        let bytes = if format == "png" {
            let idx = usize::try_from(page.saturating_sub(1)).unwrap_or(0);
            wordcraft_engine::io_ext::render_png(&app.session.doc, idx, scale, &opts)?
        } else {
            wordcraft_engine::io::save_bytes_opts(&doc_name, &app.session.doc, &opts)?
        };
        Ok(ExportResult {
            name: doc_name,
            mime: export_mime(format).to_string(),
            bytes,
        })
    }

    impl WebShell {
        fn handle_actions(&mut self, ctx: &egui::Context) {
            for action in self.actions.drain() {
                match action {
                    HostAction::SetTheme(dark) => {
                        *self.dark_state.lock().unwrap_or_else(|e| e.into_inner()) = dark;
                        let mode = if dark { "dark" } else { "light" };
                        let _ = self.app.run("ui.theme", json!({ "value": mode }));
                        theme::apply_palette(ctx, dark);
                        theme::sync_document_theme(dark);
                        host_bridge::emit_theme(&self.canvas, dark);
                    }
                    HostAction::Export {
                        format,
                        name,
                        page,
                        scale,
                        deliver_save,
                        slot,
                    } => {
                        let result = perform_export(&self.app, &format, name, page, scale);
                        if deliver_save && let Ok(ref res) = result {
                            host_bridge::deliver_save(&self.canvas, &res.name, false, &res.bytes);
                        }
                        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                    }
                }
            }
        }
    }

    impl eframe::App for WebShell {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            self.handle_actions(ctx);
            self.app.logic(ctx);
            let dark = *self.dark_state.lock().unwrap_or_else(|e| e.into_inner());
            if ctx.style().visuals.dark_mode != dark {
                theme::apply_palette(ctx, dark);
            }
            self.settings.lock().tick(web_time::Instant::now());
        }

        fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
            for f in std::mem::take(&mut raw.dropped_files) {
                let sender = self.sender.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped.docx".into());
                    if let Ok(bytes) = f.bytes_async().await {
                        let file = craft::platform::open::DroppedFile {
                            path: None,
                            name,
                            bytes: Some(Arc::from(bytes)),
                        };
                        sender.dropped(vec![file]);
                    }
                });
            }
            embed::before_input(ctx, |out| self.app.frame_input(out));
        }

        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            self.app.ui(ui);
        }
    }

    fn open_settings() -> SharedSettings<Prefs> {
        let options = OpenOptions::new(Location::Dir("fogbreak-word".into()))
            .files(Arc::new(LocalStorageFiles::new(MAX_STORED_PREFS)))
            .debounce(std::time::Duration::ZERO);
        let (store, _report) = SettingsStore::open_with(options);
        SharedSettings::new(store)
    }

    fn finish_boot(document: &web_sys::Document, canvas: &HtmlCanvasElement, result: Result<(), eframe::Error>) {
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match &result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>Fogbreak Word failed to start: {e:?}</p>")),
            }
        }
        match &result {
            Ok(()) => host_bridge::emit_ready(canvas),
            Err(e) => host_bridge::emit_failed(canvas, &format!("{e:?}")),
        }
    }

    fn build_shell(
        cc: &eframe::CreationContext<'_>,
        canvas: HtmlCanvasElement,
        req_tx: std::sync::mpsc::Sender<ControlRequest>,
        req_rx: std::sync::mpsc::Receiver<ControlRequest>,
        actions: ActionQueue,
        dark_state: Arc<Mutex<bool>>,
    ) -> WebShell {
        let ctx = cc.egui_ctx.clone();
        let dark = *dark_state.lock().unwrap_or_else(|e| e.into_inner());
        theme::apply_palette(&ctx, dark);
        theme::sync_document_theme(dark);

        let frame_ctx = ctx.clone();
        let copy_ctx = ctx.clone();
        let event_clipboard = EventClipboard::new(move || frame_ctx.cumulative_frame_nr(), move |flavors| copy_out(&copy_ctx, flavors))
            .with_wake(waker(&ctx));
        let _ = listen_for_pasted_pictures(event_clipboard.late_offers());

        let region = wordcraft_ui_egui::i18n::system_region();
        let settings = open_settings();
        let lang = wordcraft_ui_egui::i18n::Lang::from_pref(&settings.lock().get().language);
        let spec = InstallSpec::new(craft::host::target::ShellInfo::current(), Arc::new(NoWindows))
            .app_id(host_bridge::ENGINE_ID)
            .system(SystemReader::start(waker(&ctx)))
            .clipboard(Clipboard::new(event_clipboard));
        let _ = embed::install(&ctx, spec, &wordcraft_ui_egui::ui_fonts(lang));

        let browser = Arc::new(BrowserFiles::new(waker(&ctx)));
        let files: Arc<dyn Files> = Arc::new(OverlayFiles::new(browser, canvas.clone()));
        let host = Arc::new(
            Host::builder(wordcraft_ui_egui::APP)
                .i18n(wordcraft_ui_egui::i18n::catalogs())
                .files(files)
                .settings(Arc::new(settings.clone()))
                .lang(lang.craft_lang())
                .build(),
        );
        let (intake, sender) = FileIntake::new(Wake::new(waker(&ctx)));
        let sample = query_string().contains("sample");
        let doc = if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_engine::blank_document(&region) };
        let mut session = Session::new(doc);
        session.system_region = region;
        let services = Services {
            print: Some(Spool::new(host.jobs_shared().clone())),
            intake: Some(intake),
            on_dirty: Some(host_bridge::dirty_reporter(canvas.clone())),
            ..Default::default()
        };
        let mut app = WordApp::new(session, services)
            .with_host(host)
            .with_settings(settings.clone())
            .with_control(ControlChannel::new(req_rx));
        app.autosave = false;
        let mode = if dark { "dark" } else { "light" };
        let _ = app.run("ui.theme", json!({ "value": mode }));
        if !sample {
            app.start_screen();
        }

        let host_ctx = HostContext {
            canvas: canvas.clone(),
            ctx,
            requests: req_tx,
            opens: sender.clone(),
            actions: actions.clone(),
            dark_state: dark_state.clone(),
        };
        host_bridge::listen_open_and_theme(&host_ctx);
        host_bridge::install_host_rpc(host_ctx);

        WebShell {
            app,
            sender,
            settings,
            actions,
            canvas,
            dark_state,
        }
    }

    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async move {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(canvas) = host_bridge::find_page_canvas() else {
            return;
        };
        let mut options = eframe::WebOptions::default();
        if query_string().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let dark = theme::initial_dark_mode();
        let dark_state = Arc::new(Mutex::new(dark));
        let actions = ActionQueue::default();
        let (req_tx, req_rx) = channel::<ControlRequest>();
        let canvas_for_boot = canvas.clone();
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    Ok(Box::new(build_shell(
                        cc,
                        canvas_for_boot.clone(),
                        req_tx.clone(),
                        req_rx,
                        actions,
                        dark_state,
                    )))
                }),
            )
            .await;
        finish_boot(&document, &canvas, result);
    });
}
