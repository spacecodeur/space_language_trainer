use anyhow::{Result, bail};
use cpal::traits::{DeviceTrait, HostTrait};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use evdev::KeyCode as EvdevKeyCode;
use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceMode {
    Manual, // Push-to-talk: hotkey toggle-off sends accumulated audio
    Auto,   // VAD auto-segmentation on silence (original behavior)
}

pub struct SetupConfig {
    pub server_addr: String,
    pub device: cpal::Device,
    pub device_name: String,
    pub hotkey: EvdevKeyCode,
    pub cancel_key: EvdevKeyCode,
    pub voice_mode: VoiceMode,
}

pub fn run_setup() -> Result<SetupConfig> {
    // Auto-detect default audio input device
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No default audio input device found."))?;
    let device_name = device
        .description()
        .map(|d: cpal::DeviceDescription| d.name().to_string())
        .unwrap_or_else(|_| "Default".into());

    let mut terminal = ratatui::init();

    // Screen 1: Server address input
    let server_addr = match text_input_screen(&mut terminal, "Server Address", "127.0.0.1:9500") {
        Ok(t) => t,
        Err(e) => {
            ratatui::restore();
            return Err(e);
        }
    };

    // Screen 2: Push-to-Talk Key selection
    let hotkey_choices = vec![
        "F2".to_string(),
        "F3".to_string(),
        "F4".to_string(),
        "F9".to_string(),
        "F10".to_string(),
        "F11".to_string(),
        "F12".to_string(),
        "ScrollLock".to_string(),
        "Pause".to_string(),
    ];
    let hotkey_idx = match select_screen(&mut terminal, "Select Push-to-Talk Key", &hotkey_choices)
    {
        Ok(idx) => idx,
        Err(e) => {
            ratatui::restore();
            return Err(e);
        }
    };

    // Screen 3: Cancel Key selection
    let cancel_choices = vec![
        "F5".to_string(),
        "F6".to_string(),
        "F7".to_string(),
        "F8".to_string(),
    ];
    let cancel_idx = match select_screen(&mut terminal, "Select Cancel Key", &cancel_choices) {
        Ok(idx) => idx,
        Err(e) => {
            ratatui::restore();
            return Err(e);
        }
    };

    // Screen 4: Voice Mode selection
    let mode_choices = vec![
        "Manual (hotkey controls when to send)".to_string(),
        "Auto (VAD segments on silence)".to_string(),
    ];
    let mode_idx = match select_screen(&mut terminal, "Select Voice Mode", &mode_choices) {
        Ok(idx) => idx,
        Err(e) => {
            ratatui::restore();
            return Err(e);
        }
    };

    ratatui::restore();

    let voice_mode = match mode_idx {
        0 => VoiceMode::Manual,
        _ => VoiceMode::Auto,
    };

    let hotkey = match hotkey_idx {
        0 => EvdevKeyCode::KEY_F2,
        1 => EvdevKeyCode::KEY_F3,
        2 => EvdevKeyCode::KEY_F4,
        3 => EvdevKeyCode::KEY_F9,
        4 => EvdevKeyCode::KEY_F10,
        5 => EvdevKeyCode::KEY_F11,
        6 => EvdevKeyCode::KEY_F12,
        7 => EvdevKeyCode::KEY_SCROLLLOCK,
        8 => EvdevKeyCode::KEY_PAUSE,
        _ => EvdevKeyCode::KEY_F2,
    };

    let cancel_key = match cancel_idx {
        0 => EvdevKeyCode::KEY_F5,
        1 => EvdevKeyCode::KEY_F6,
        2 => EvdevKeyCode::KEY_F7,
        3 => EvdevKeyCode::KEY_F8,
        _ => EvdevKeyCode::KEY_F5,
    };

    Ok(SetupConfig {
        server_addr,
        device,
        device_name,
        hotkey,
        cancel_key,
        voice_mode,
    })
}

fn text_input_screen(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
    placeholder: &str,
) -> Result<String> {
    let mut input = String::new();

    loop {
        let display_text = if input.is_empty() {
            placeholder.to_string()
        } else {
            input.clone()
        };
        let is_empty = input.is_empty();
        let title = format!(" {title} (Enter=confirm, Esc=cancel) ");

        terminal.draw(|frame: &mut Frame| {
            let area = frame.area();
            let style = if is_empty {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };
            let paragraph = Paragraph::new(format!("{display_text}_"))
                .style(style)
                .block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(paragraph, area);
        })?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char(c) => input.push(c),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Enter => {
                    let trimmed = input.trim().to_string();
                    // If empty, use placeholder as default
                    if trimmed.is_empty() {
                        return Ok(placeholder.to_string());
                    }
                    return Ok(trimmed);
                }
                KeyCode::Esc => {
                    bail!("Setup cancelled by user.");
                }
                _ => {}
            }
        }
    }
}

fn select_screen(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
    items: &[String],
) -> Result<usize> {
    let mut state = ListState::default();
    state.select(Some(0));

    loop {
        let title = title.to_string();
        let list_items: Vec<ListItem> = items.iter().map(|s| ListItem::new(s.as_str())).collect();

        terminal.draw(|frame: &mut Frame| {
            let area = frame.area();
            let list = List::new(list_items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {title} (↑↓ Enter, q=quit) ")),
                )
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("▸ ");
            frame.render_stateful_widget(list, area, &mut state);
        })?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Up => state.select_previous(),
                KeyCode::Down => state.select_next(),
                KeyCode::Enter => {
                    if let Some(idx) = state.selected() {
                        return Ok(idx);
                    }
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    bail!("Setup cancelled by user.");
                }
                _ => {}
            }
        }
    }
}
