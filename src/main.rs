mod api;
mod app;
mod config;
mod img;
mod ui;
mod video;

// in retrospect sshgoon would be a funnier name. oh well. the only remnant you're getting of that name is this niche code comment. and this commit.
// look at this nerd reading the source code. would you really not trust ME! 
use std::io;
use std::time::Duration;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use ratatui::prelude::*;
use ratatui_image::picker::{Picker, ProtocolType};
use app::{App, Screen, InputTarget};
use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load();

    // Must enter raw mode before querying the terminal for graphics capabilities:
    // the capability query sends escape sequences and reads the response byte-by-byte,
    // which requires raw (non-line-buffered) mode to work reliably.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let picker = match Picker::from_query_stdio() {
        Ok(p) if p.protocol_type() != ProtocolType::Halfblocks => Some(p),
        _ => None,
    };

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(picker, cfg);
    let res = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    if let Err(e) = res {
        eprintln!("Error: {e}");
    }
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // When a video is animating, wake up exactly when the next frame is due
        // (frame interval minus the time already spent drawing this iteration),
        // not a full interval *after* the draw. Otherwise stay responsive enough
        // to pick up a finished video decode.
        let timeout = if app.is_animating() {
            let interval = Duration::from_secs_f64(app.frame_interval());
            interval.checked_sub(app.time_since_tick()).unwrap_or(Duration::ZERO)
        } else if app.video_loader.is_some() || app.encode_loader.is_some() {
            Duration::from_millis(60)
        } else {
            // Idle: block until input (resize/key) so we don't repeatedly
            // re-emit a static graphics image, which can flicker.
            Duration::from_secs(3600)
        };

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return Ok(());
                }

                match app.screen {
                    Screen::Search => handle_search(app, key.code),
                    Screen::Results => handle_results(app, key.code),
                    Screen::Detail => handle_detail(app, key.code),
                    Screen::Settings => handle_settings(app, key.code),
                    Screen::Help => {
                        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
                            app.screen = app.prev_screen.clone();
                        }
                    }
                }

                if app.should_quit {
                    return Ok(());
                }
            }
        }

        // Pick up a finished background video decode, then a finished encode.
        app.poll_video();
        app.poll_encode();

        // Keep audio playback in step with the on-screen animation.
        app.sync_audio();

        // Advance the video preview on its own simple fps clock. Audio runs
        // independently on the rodio thread; we deliberately don't sync them.
        if app.is_animating() {
            app.tick();
        }
    }
}

fn handle_search(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Tab => {
            app.input_target = match app.input_target {
                InputTarget::Tags => InputTarget::Sort,
                InputTarget::Sort => InputTarget::Rating,
                InputTarget::Rating => InputTarget::Tags,
            };
        }
        KeyCode::Enter => {
            app.search();
        }
        KeyCode::Char('?') if !matches!(app.input_target, InputTarget::Tags) => {
            app.prev_screen = app.screen.clone();
            app.screen = Screen::Help;
        }
        KeyCode::Char(c) => {
            if matches!(app.input_target, InputTarget::Tags) {
                app.tag_input.push(c);
            } else if matches!(app.input_target, InputTarget::Sort) {
                app.cycle_sort();
            } else {
                app.cycle_rating();
            }
        }
        KeyCode::Backspace => {
            if matches!(app.input_target, InputTarget::Tags) {
                app.tag_input.pop();
            }
        }
        _ => {}
    }
}

fn handle_results(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = Screen::Search;
        }
        KeyCode::Up | KeyCode::Char('k') => app.prev_post(),
        KeyCode::Down | KeyCode::Char('j') => app.next_post(),
        KeyCode::Enter => {
            if !app.posts.is_empty() {
                app.screen = Screen::Detail;
            }
        }
        KeyCode::Char('n') => app.next_page(),
        KeyCode::Char('p') => app.prev_page(),
        KeyCode::Char('o') => app.open_in_browser(),
        KeyCode::Char('d') => app.download_current(),
        KeyCode::Char('i') => app.toggle_image(),
        KeyCode::Char('s') => app.open_settings(),
        KeyCode::Char('?') => {
            app.prev_screen = app.screen.clone();
            app.screen = Screen::Help;
        }
        _ => {}
    }
}

fn handle_detail(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = Screen::Results;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.detail_scroll > 0 {
                app.detail_scroll -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.detail_scroll += 1;
        }
        KeyCode::Char('o') => app.open_in_browser(),
        KeyCode::Char('d') => app.download_current(),
        KeyCode::Left | KeyCode::Char('h') => {
            app.prev_post();
            app.detail_scroll = 0;
            if app.show_image { app.load_image_for_current(); }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.next_post();
            app.detail_scroll = 0;
            if app.show_image { app.load_image_for_current(); }
        }
        KeyCode::Char('i') => app.toggle_image(),
        KeyCode::Char('s') => app.open_settings(),
        KeyCode::Char('?') => {
            app.prev_screen = app.screen.clone();
            app.screen = Screen::Help;
        }
        _ => {}
    }
}

fn handle_settings(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.screen = app.prev_screen.clone(),
        KeyCode::Enter => app.save_settings(),
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('+') | KeyCode::Char('=') => app.adjust_fps(1),
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('-') | KeyCode::Char('_') => app.adjust_fps(-1),
        _ => {}
    }
}
