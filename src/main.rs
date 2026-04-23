use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::{
    error::Error,
    fs, io,
    path::PathBuf,
    process::Command,
    sync::mpsc::{channel, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

struct App {
    input: String,
    input_mode: InputMode,
    should_quit: bool,
    status: String,
    url: String,
    duration_secs: f64,
    range_start: f64,
    range_end: f64,
    interval_secs: f64,
    output_dir: String,
    busy: Option<BusyState>,
    spinner_idx: usize,
}

struct BusyState {
    rx: Receiver<WorkerMsg>,
    label: String,
}

enum WorkerMsg {
    Status(String),
    DurationDone(Result<f64, String>),
    ProcessDone(Result<(usize, String), String>),
}

#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Normal,
    EditingUrl,
    AskingRange,
    AskingInterval,
    AskingPath,
    Busy,
    Ready,
}

impl Default for App {
    fn default() -> App {
        App {
            input: String::new(),
            input_mode: InputMode::Normal,
            should_quit: false,
            status: String::new(),
            url: String::new(),
            duration_secs: 0.0,
            range_start: 0.0,
            range_end: 0.0,
            interval_secs: 0.0,
            output_dir: String::new(),
            busy: None,
            spinner_idx: 0,
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = App::default();
    let res = run_app(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{err:?}");
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<(), Box<dyn Error>>
where
    <B as Backend>::Error: 'static,
{
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(100);
    loop {
        terminal.draw(|f| ui(f, &app))?;

        // Poll worker progress
        if let Some(busy) = app.busy.as_mut() {
            loop {
                match busy.rx.try_recv() {
                    Ok(WorkerMsg::Status(s)) => {
                        busy.label = s;
                    }
                    Ok(WorkerMsg::DurationDone(res)) => {
                        match res {
                            Ok(secs) => {
                                app.duration_secs = secs;
                                app.status = format!(
                                    "Duración: {} ({:.0} s)",
                                    format_hms(secs),
                                    secs
                                );
                                app.input.clear();
                                app.input_mode = InputMode::AskingRange;
                            }
                            Err(e) => {
                                app.status = format!("Error al obtener duración: {e}");
                                app.input_mode = InputMode::EditingUrl;
                            }
                        }
                        app.busy = None;
                        break;
                    }
                    Ok(WorkerMsg::ProcessDone(res)) => {
                        match res {
                            Ok((count, dir)) => {
                                app.status = format!(
                                    "Listo. {count} frames nuevos en {dir} (rango {:.0}-{:.0}s, cada {}s).",
                                    app.range_start, app.range_end, app.interval_secs
                                );
                            }
                            Err(e) => {
                                app.status = format!("Error: {e}");
                            }
                        }
                        app.busy = None;
                        app.input_mode = InputMode::Ready;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        app.status = "Worker desconectado inesperadamente.".into();
                        app.busy = None;
                        app.input_mode = InputMode::Ready;
                        break;
                    }
                }
            }
        }

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if !event::poll(timeout)? {
            if last_tick.elapsed() >= tick_rate {
                app.spinner_idx = app.spinner_idx.wrapping_add(1);
                last_tick = Instant::now();
            }
            continue;
        }

        let event = event::read()?;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('e') => {
                            app.input_mode = InputMode::EditingUrl;
                            app.input.clear();
                        }
                        KeyCode::Char('q') => {
                            app.should_quit = true;
                        }
                        _ => {}
                    },
                    InputMode::Ready => match key.code {
                        KeyCode::Char('q') => app.should_quit = true,
                        KeyCode::Char('r') => {
                            app.input_mode = InputMode::Normal;
                            app.status.clear();
                        }
                        _ => {}
                    },
                    InputMode::Busy => {}
                    _ => match key.code {
                        KeyCode::Enter => handle_enter(&mut app, terminal)?,
                        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                if let Ok(text) = cb.get_text() {
                                    app.input.push_str(&text);
                                }
                            }
                        }
                        KeyCode::Char(c) if c != '\n' && c != '\r' => {
                            app.input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                            app.input.clear();
                            app.status = "Cancelado.".into();
                        }
                        _ => {}
                    },
                }
            }
            Event::Paste(text) => {
                if matches!(
                    app.input_mode,
                    InputMode::EditingUrl
                        | InputMode::AskingRange
                        | InputMode::AskingInterval
                        | InputMode::AskingPath
                ) {
                    let cleaned: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                    app.input.push_str(&cleaned);
                }
            }
            _ => {}
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_enter<B: Backend>(app: &mut App, _terminal: &mut Terminal<B>) -> Result<(), Box<dyn Error>>
where
    <B as Backend>::Error: 'static,
{
    match app.input_mode {
        InputMode::EditingUrl => {
            if app.input.trim().is_empty() {
                return Ok(());
            }
            app.url = app.input.trim().to_string();
            let (tx, rx) = channel::<WorkerMsg>();
            let url = app.url.clone();
            thread::spawn(move || {
                let _ = tx.send(WorkerMsg::Status("Consultando yt-dlp".into()));
                let res = fetch_duration_secs(&url);
                let _ = tx.send(WorkerMsg::DurationDone(res));
            });
            app.busy = Some(BusyState {
                rx,
                label: "Analizando duración".into(),
            });
            app.input_mode = InputMode::Busy;
            app.status.clear();
        }
        InputMode::AskingRange => {
            let trimmed = app.input.trim();
            if trimmed.is_empty() {
                app.range_start = 0.0;
                app.range_end = app.duration_secs;
            } else {
                match parse_range(trimmed, app.duration_secs) {
                    Ok((s, e)) => {
                        app.range_start = s;
                        app.range_end = e;
                    }
                    Err(msg) => {
                        app.status = format!("Rango inválido: {msg}");
                        return Ok(());
                    }
                }
            }
            app.status = format!(
                "Rango: {:.0}s - {:.0}s",
                app.range_start, app.range_end
            );
            app.input.clear();
            app.input_mode = InputMode::AskingInterval;
        }
        InputMode::AskingInterval => {
            let trimmed = app.input.trim();
            match trimmed.parse::<f64>() {
                Ok(v) if v > 0.0 => {
                    app.interval_secs = v;
                    app.status = format!("Intervalo: cada {v}s");
                    app.input.clear();
                    app.input_mode = InputMode::AskingPath;
                }
                _ => {
                    app.status = "Intervalo inválido: ingresa un número mayor a 0.".into();
                }
            }
        }
        InputMode::AskingPath => {
            let trimmed = app.input.trim();
            if trimmed.is_empty() {
                app.status = "Ruta inválida: no puede estar vacía.".into();
                return Ok(());
            }
            app.output_dir = trimmed.to_string();
            app.input.clear();

            if let Err(e) = fs::create_dir_all(&app.output_dir) {
                app.status = format!("No se pudo crear la carpeta: {e}");
                return Ok(());
            }

            let (tx, rx) = channel::<WorkerMsg>();
            let url = app.url.clone();
            let parent = app.output_dir.clone();
            let start = app.range_start;
            let end = app.range_end;
            let interval = app.interval_secs;

            thread::spawn(move || {
                let _ = tx.send(WorkerMsg::Status("Obteniendo ID del video".into()));
                let video_id = match fetch_video_id(&url) {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = tx.send(WorkerMsg::ProcessDone(Err(format!("ID: {e}"))));
                        return;
                    }
                };
                let parent_clean = parent.trim_end_matches(['/', '\\']).to_string();
                let video_dir = format!("{parent_clean}/{video_id}");
                if let Err(e) = fs::create_dir_all(&video_dir) {
                    let _ = tx.send(WorkerMsg::ProcessDone(Err(format!("carpeta: {e}"))));
                    return;
                }
                let _ = tx.send(WorkerMsg::Status(format!("Descargando en {video_dir}")));
                let video_path = match download_video(&url, &video_dir, &video_id) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = tx.send(WorkerMsg::ProcessDone(Err(format!("descarga: {e}"))));
                        return;
                    }
                };
                let _ = tx.send(WorkerMsg::Status("Extrayendo frames".into()));
                match extract_frames(&video_path, start, end, interval, &video_dir) {
                    Ok(count) => {
                        let _ = tx.send(WorkerMsg::ProcessDone(Ok((count, video_dir))));
                    }
                    Err(e) => {
                        let _ = tx.send(WorkerMsg::ProcessDone(Err(format!("ffmpeg: {e}"))));
                    }
                }
            });

            app.busy = Some(BusyState {
                rx,
                label: "Iniciando".into(),
            });
            app.input_mode = InputMode::Busy;
            app.status.clear();
        }
        _ => {}
    }
    Ok(())
}

fn fetch_duration_secs(url: &str) -> Result<f64, String> {
    let output = Command::new("yt-dlp")
        .args(["--print", "duration", "--no-warnings", "--no-playlist", url])
        .output()
        .map_err(|e| format!("no se pudo ejecutar yt-dlp ({e})"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() { "yt-dlp falló".into() } else { stderr });
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    raw.parse::<f64>()
        .map_err(|_| format!("yt-dlp devolvió un valor inesperado: {raw}"))
}

fn fetch_video_id(url: &str) -> Result<String, String> {
    let id_out = Command::new("yt-dlp")
        .args(["--get-id", "--no-warnings", "--no-playlist", url])
        .output()
        .map_err(|e| format!("no se pudo ejecutar yt-dlp ({e})"))?;
    if !id_out.status.success() {
        let stderr = String::from_utf8_lossy(&id_out.stderr).trim().to_string();
        return Err(if stderr.is_empty() { "yt-dlp falló al obtener ID".into() } else { stderr });
    }
    let video_id = String::from_utf8_lossy(&id_out.stdout).trim().to_string();
    if video_id.is_empty() {
        Err("yt-dlp no devolvió ID del video".into())
    } else {
        Ok(video_id)
    }
}

fn download_video(url: &str, output_dir: &str, video_id: &str) -> Result<PathBuf, String> {
    let dir_clean = output_dir.trim_end_matches(['/', '\\']);
    let template = format!("{dir_clean}/{video_id}.%(ext)s");
    let output = Command::new("yt-dlp")
        .args([
            "-f",
            "best[ext=mp4]/best",
            "--no-warnings", "--no-playlist",
            "-o",
            &template,
            url,
        ])
        .output()
        .map_err(|e| format!("no se pudo ejecutar yt-dlp ({e})"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() { "yt-dlp falló".into() } else { stderr });
    }
    for entry in fs::read_dir(output_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.file_stem().and_then(|s| s.to_str()) == Some(video_id) {
            return Ok(path);
        }
    }
    Err("no se encontró el archivo descargado".into())
}

fn extract_frames(
    video: &PathBuf,
    start: f64,
    end: f64,
    interval: f64,
    output_dir: &str,
) -> Result<usize, String> {
    let dir_clean = output_dir.trim_end_matches(['/', '\\']);
    let start_idx = next_frame_index(dir_clean);
    let pattern = format!("{dir_clean}/frame_%04d.jpg");
    let fps = format!("fps=1/{interval}");
    let start_s = format!("{start}");
    let end_s = format!("{end}");
    let start_number = format!("{start_idx}");
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-ss",
            &start_s,
            "-to",
            &end_s,
            "-i",
        ])
        .arg(video)
        .args([
            "-vf",
            &fps,
            "-q:v",
            "2",
            "-start_number",
            &start_number,
            &pattern,
        ])
        .output()
        .map_err(|e| format!("no se pudo ejecutar ffmpeg ({e})"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        return Err(if tail.is_empty() { "ffmpeg falló".into() } else { tail });
    }
    let total = fs::read_dir(output_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with("frame_") && s.ends_with(".jpg"))
                .unwrap_or(false)
        })
        .count();
    let new_count = total.saturating_sub(start_idx - 1);
    Ok(new_count)
}

fn next_frame_index(dir: &str) -> usize {
    let mut max_idx = 0usize;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.path().file_name().and_then(|s| s.to_str()) {
                if let Some(stem) = name.strip_prefix("frame_").and_then(|s| s.strip_suffix(".jpg")) {
                    if let Ok(n) = stem.parse::<usize>() {
                        if n > max_idx {
                            max_idx = n;
                        }
                    }
                }
            }
        }
    }
    max_idx + 1
}

fn format_hms(secs: f64) -> String {
    let s = secs as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

fn parse_time(s: &str) -> Result<f64, String> {
    if s.contains(':') {
        let mut total = 0.0;
        for part in s.split(':') {
            let v: f64 = part.parse().map_err(|_| format!("'{part}' no es número"))?;
            total = total * 60.0 + v;
        }
        Ok(total)
    } else {
        s.parse::<f64>().map_err(|_| format!("'{s}' no es número"))
    }
}

fn parse_range(input: &str, duration: f64) -> Result<(f64, f64), String> {
    let parts: Vec<&str> = input.split('-').map(|p| p.trim()).collect();
    if parts.len() != 2 {
        return Err("usa formato inicio-fin, p.ej. 10-30".into());
    }
    let start = parse_time(parts[0])?;
    let end = parse_time(parts[1])?;
    if start < 0.0 || end <= start {
        return Err("inicio debe ser < fin y >= 0".into());
    }
    if end > duration {
        return Err(format!("fin excede duración ({:.0}s)", duration));
    }
    Ok((start, end))
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Length(3), // Title
                Constraint::Length(3), // Input
                Constraint::Length(3), // Status
                Constraint::Min(1),    // Help text
            ]
            .as_ref(),
        )
        .split(f.area());

    let title = Paragraph::new("🎬 YouTube Frame Extractor by Moibe")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    let spinner_frames = ['|', '/', '-', '\\'];
    let spinner = spinner_frames[app.spinner_idx % spinner_frames.len()];
    let busy_text = app
        .busy
        .as_ref()
        .map(|b| format!("{spinner} {}...", b.label))
        .unwrap_or_default();

    let (input_title, input_text): (&str, &str) = match app.input_mode {
        InputMode::Normal => ("Acción", "Presiona 'e' para ingresar URL, 'q' para salir"),
        InputMode::EditingUrl => ("YouTube URL", app.input.as_str()),
        InputMode::AskingRange => (
            "[1/3] Rango en segundos (inicio-fin, Enter=todo)",
            app.input.as_str(),
        ),
        InputMode::AskingInterval => (
            "[2/3] ¿Cada cuántos segundos una captura?",
            app.input.as_str(),
        ),
        InputMode::AskingPath => (
            "[3/3] Ruta de la carpeta padre donde guardar",
            app.input.as_str(),
        ),
        InputMode::Busy => ("Procesando", busy_text.as_str()),
        InputMode::Ready => ("Listo", "Presiona 'r' para reiniciar, 'q' para salir"),
    };

    let is_editing = matches!(
        app.input_mode,
        InputMode::EditingUrl | InputMode::AskingRange | InputMode::AskingInterval | InputMode::AskingPath
    );

    let input = Paragraph::new(input_text)
        .style(if is_editing {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        })
        .block(Block::default().borders(Borders::ALL).title(input_title));
    f.render_widget(input, chunks[1]);

    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(Color::Magenta))
        .block(Block::default().borders(Borders::ALL).title("Estado"));
    f.render_widget(status, chunks[2]);

    let help_text = vec![
        Line::from(vec![
            Span::styled("e", Style::default().fg(Color::Green)),
            Span::raw(" - Iniciar (ingresar URL)"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+V", Style::default().fg(Color::Green)),
            Span::raw(" - Pegar"),
        ]),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" - Confirmar cada pregunta"),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Green)),
            Span::raw(" - Cancelar"),
        ]),
        Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Green)),
            Span::raw(" - Salir"),
        ]),
    ];

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Ayuda"));
    f.render_widget(help, chunks[3]);

    if is_editing {
        f.set_cursor_position((
            chunks[1].x + app.input.len() as u16 + 1,
            chunks[1].y + 1,
        ));
    }
}
