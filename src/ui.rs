use anyhow::Result;
use crossterm::{
    event::{self, Event as CEvent, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use kube::api::Meta;
use std::time::Duration;
use std::{io, time::Instant};
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{
        Block, BorderType, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Tabs,
    },
    Terminal,
};

use crate::{util::get_pods, UIOpts};

#[derive(Clone, Debug)]
enum Event<I> {
    Input(I),
    Tick,
}

#[derive(Copy, Clone, Debug)]
enum MenuItem {
    Home,
}

impl From<MenuItem> for usize {
    fn from(input: MenuItem) -> usize {
        match input {
            MenuItem::Home => 0,
        }
    }
}

#[derive(Clone, Debug)]
struct KubePod {
    name: String,
}

pub async fn load_ui(namespace: &str, _opts: &UIOpts) -> Result<()> {
    println!("Loading UI...");
    enable_raw_mode().expect("can run in raw mode");

    let (mut tx, mut rx) = tokio::sync::mpsc::channel(1);
    let tick_rate = Duration::from_millis(200);
    tokio::spawn(async move {
        let mut last_tick = Instant::now();
        loop {
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if event::poll(timeout).expect("poll works") {
                if let CEvent::Key(key) = event::read().expect("can read events") {
                    let _ = tx.send(Event::Input(key)).await;
                }
            }

            if last_tick.elapsed() >= tick_rate {
                if let Ok(_) = tx.send(Event::Tick).await {
                    last_tick = Instant::now();
                }
            }
        }
    });

    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let menu_titles = vec!["Home"];
    let active_menu_item = MenuItem::Home;
    let mut pod_list_state = ListState::default();
    pod_list_state.select(Some(0));

    let pod_list = refresh_pod_list(namespace).await?;

    loop {
        terminal.draw(|rect| {
            let size = rect.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(2),
                        Constraint::Length(3),
                    ]
                    .as_ref(),
                )
                .split(size);

            let copyright = Paragraph::new("ice-kube 2021")
                .style(Style::default().fg(Color::LightCyan))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::White))
                        .title("Copyright")
                        .border_type(BorderType::Plain),
                );

            let menu = menu_titles
                .iter()
                .map(|t| {
                    let (first, rest) = t.split_at(1);
                    Spans::from(vec![
                        Span::styled(
                            first,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                })
                .collect();

            let tabs = Tabs::new(menu)
                .select(active_menu_item.into())
                .block(Block::default().title("Menu").borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .highlight_style(Style::default().fg(Color::Yellow))
                .divider(Span::raw("|"));

            rect.render_widget(tabs, chunks[0]);
            match active_menu_item {
                MenuItem::Home => {
                    let pods_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(
                            [Constraint::Percentage(20), Constraint::Percentage(80)].as_ref(),
                        )
                        .split(chunks[1]);
                    let (left, right) = render_pods(&pod_list, &pod_list_state);

                    rect.render_stateful_widget(left, pods_chunks[0], &mut pod_list_state);
                    rect.render_widget(right, pods_chunks[1]);
                }
            }
            rect.render_widget(copyright, chunks[2]);
        })?;

        if let Some(event) = rx.recv().await {
            match event {
                Event::Input(event) => match event.code {
                    KeyCode::Char('q') => {
                        disable_raw_mode()?;
                        terminal.show_cursor()?;
                        break;
                    }
                    // KeyCode::Char('h') => active_menu_item = MenuItem::Home,
                    // KeyCode::Char('p') => active_menu_item = MenuItem::Pets,
                    // KeyCode::Char('a') => {
                    //     add_random_pet_to_db().expect("can add new random pet");
                    // }
                    // KeyCode::Char('d') => {
                    //     remove_pet_at_index(&mut pet_list_state).expect("can remove pet");
                    // }
                    // KeyCode::Down => {
                    //     if let Some(selected) = pet_list_state.selected() {
                    //         let amount_pets = read_db().expect("can fetch pet list").len();
                    //         if selected >= amount_pets - 1 {
                    //             pet_list_state.select(Some(0));
                    //         } else {
                    //             pet_list_state.select(Some(selected + 1));
                    //         }
                    //     }
                    // }
                    // KeyCode::Up => {
                    //     if let Some(selected) = pet_list_state.selected() {
                    //         let amount_pets = read_db().expect("can fetch pet list").len();
                    //         if selected > 0 {
                    //             pet_list_state.select(Some(selected - 1));
                    //         } else {
                    //             pet_list_state.select(Some(amount_pets - 1));
                    //         }
                    //     }
                    // }
                    _ => {}
                },
                Event::Tick => {}
            }
        }
    }

    Ok(())
}

async fn refresh_pod_list(namespace: &str) -> Result<Vec<KubePod>> {
    let pod_list: Vec<_> = get_pods(namespace)
        .await?
        .iter()
        .map(|p| KubePod {
            name: Meta::name(p),
        })
        .collect();

    Ok(pod_list)
}

fn render_pods<'a>(pod_list: &[KubePod], pod_list_state: &ListState) -> (List<'a>, Table<'a>) {
    let pods = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White))
        .title("Pods")
        .border_type(BorderType::Plain);

    let items: Vec<_> = pod_list
        .iter()
        .map(|pod| {
            ListItem::new(Spans::from(vec![Span::styled(
                pod.name.clone(),
                Style::default(),
            )]))
        })
        .collect();

    let selected_pod = pod_list
        .get(
            pod_list_state
                .selected()
                .expect("there is always a selected pod"),
        )
        .expect("exists")
        .clone();

    let list = List::new(items).block(pods).highlight_style(
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    );

    let pod_detail = Table::new(vec![Row::new(vec![
        Cell::from(Span::raw(selected_pod.name)),
        Cell::from(Span::raw("1/1")),
        Cell::from(Span::raw("0")),
        Cell::from(Span::raw("Running")),
    ])])
    .header(Row::new(vec![
        Cell::from(Span::styled(
            "Name",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Ready",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Restarts",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Status",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White))
            .title("Detail")
            .border_type(BorderType::Plain),
    )
    .widths(&[
        Constraint::Percentage(20),
        Constraint::Percentage(5),
        Constraint::Percentage(5),
        Constraint::Percentage(70),
    ]);

    (list, pod_detail)
}
