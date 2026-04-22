use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
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
use std::{error::Error, io};

struct App {
    input: String,
    input_mode: InputMode,
    should_quit: bool,
}

#[derive(Clone, Copy)]
enum InputMode {
    Normal,
    Editing,
}

impl Default for App {
    fn default() -> App {
        App {
            input: String::new(),
            input_mode: InputMode::Normal,
            should_quit: false,
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run it
    let app = App::default();
    let res = run_app(&mut terminal, app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
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
    loop {
        terminal.draw(|f| ui(f, &app))?;

        let event = event::read()?;
        match event {
            Event::Key(key) => {
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('e') => {
                            app.input_mode = InputMode::Editing;
                        }
                        KeyCode::Char('q') => {
                            app.should_quit = true;
                        }
                        _ => {}
                    },
                    InputMode::Editing => match key.code {
                        KeyCode::Enter => {
                            if !app.input.trim().is_empty() {
                                // Here we'll add the video processing logic
                                println!("Processing URL: {}", app.input);
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        _ => {}
                    },
                }
            }
            Event::Paste(text) => {
                if let InputMode::Editing = app.input_mode {
                    app.input.push_str(&text);
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

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Length(3), // Title
                Constraint::Length(3), // Input
                Constraint::Min(1),    // Help text
            ]
            .as_ref(),
        )
        .split(f.area());

    let title = Paragraph::new("🎬 YouTube Frame Extractor by Moibe")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    let input_text = match app.input_mode {
        InputMode::Normal => "Press 'e' to enter URL, 'q' to quit",
        InputMode::Editing => &app.input,
    };

    let input = Paragraph::new(input_text)
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default().fg(Color::Yellow),
        })
        .block(Block::default().borders(Borders::ALL).title("YouTube URL"));
    f.render_widget(input, chunks[1]);

    let help_text = vec![
        Line::from(vec![
            Span::styled("e", Style::default().fg(Color::Green)),
            Span::raw(" - Enter URL editing mode"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+V", Style::default().fg(Color::Green)),
            Span::raw(" - Paste URL (in editing mode)"),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Green)),
            Span::raw(" - Exit editing mode"),
        ]),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" - Process video (when URL entered)"),
        ]),
        Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Green)),
            Span::raw(" - Quit application"),
        ]),
    ];

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"));
    f.render_widget(help, chunks[2]);

    // Show the cursor in the input field
    if let InputMode::Editing = app.input_mode {
        f.set_cursor_position((
            chunks[1].x + app.input.len() as u16 + 1,
            chunks[1].y + 1,
        ));
    }
}
