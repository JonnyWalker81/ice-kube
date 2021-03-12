use anyhow::Result;
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use k8s_openapi::api::core::v1::Pod;
use kube::api::Meta;
use std::{io, time::Instant};
use std::{panic, time::Duration};
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState, Tabs},
    Terminal,
};

use crate::{
    util::{describe_pod, get_context, get_pods},
    UIOpts,
};

#[derive(Clone, Debug)]
pub enum Event<I> {
    Input(I),
    Tick,
}

#[derive(Copy, Clone, Debug)]
enum ActionItem {
    Home,
}

impl From<ActionItem> for usize {
    fn from(input: ActionItem) -> usize {
        match input {
            ActionItem::Home => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KubePod {
    name: String,
    ready: i32,
    container_count: i32,
    status: String,
    restart_count: i32,
}

#[derive(Clone, Debug)]
pub enum UIEvent {
    RefreshPods(Vec<KubePod>),
}

// #[derive(Clone, Debug)]
// pub struct UI {
//     pub event_tx: Option<tokio::sync::mpsc::Sender<Event<KeyEvent>>>,
// }

// impl UI {
//     pub fn new(tx: tokio::sync::mpsc::Sender<Event<KeyEvent>>) -> Self {
//         Self { event_tx: Some(tx) }
//     }
// }

fn start_key_events() -> tokio::sync::mpsc::Receiver<Event<KeyEvent>> {
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

    rx
}

pub async fn load_ui(namespace: &str, _opts: &UIOpts) -> Result<()> {
    println!("Loading UI...");
    enable_raw_mode().expect("can run in raw mode");

    panic::set_hook(Box::new(|info| {
        println!("Panic: {}", info);
        disable_raw_mode().expect("restore terminal raw mode");
    }));

    let mut rx = start_key_events();

    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    io::stdout().execute(EnterAlternateScreen)?;
    terminal.clear()?;

    let menu_titles = vec!["Pods"];
    let mut active_action_item = ActionItem::Home;
    let mut pod_table_state = TableState::default();
    pod_table_state.select(Some(0));

    let (mut ui_tx, mut ui_rx) = tokio::sync::mpsc::channel(1);
    let mut pod_list = vec![];
    refresh_pod_list(namespace, ui_tx.clone());
    let cluster_url = get_context().await?;

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

            let cluster_context = Paragraph::new(cluster_url.to_string())
                .style(Style::default().fg(Color::LightCyan))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::White))
                        .title("Context")
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
                .select(active_action_item.into())
                .block(Block::default().title("Menu").borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .highlight_style(Style::default().fg(Color::Yellow))
                .divider(Span::raw("|"));

            rect.render_widget(tabs, chunks[0]);
            match active_action_item {
                ActionItem::Home => {
                    let table = render_pods(&pod_list);
                    rect.render_stateful_widget(table, chunks[1], &mut pod_table_state);
                }
            }
            rect.render_widget(cluster_context, chunks[2]);
        })?;

        tokio::select! {
        Some(event) = rx.recv() =>{
               match event {
                   Event::Input(event) => match event.code {
                       KeyCode::Char('q') => {
                           disable_raw_mode()?;
                           io::stdout().execute(LeaveAlternateScreen)?;
                           terminal.show_cursor()?;
                           break;
                       }
                       KeyCode::Down | KeyCode::Char('j') => match active_action_item {
                           ActionItem::Home => {
                               if let Some(selected) = pod_table_state.selected() {
                                   if selected >= pod_list.len() - 1 {
                                       pod_table_state.select(Some(0));
                                   } else {
                                       pod_table_state.select(Some(selected + 1));
                                   }
                               }
                           }
                       },
                       KeyCode::Up | KeyCode::Char('k') => match active_action_item {
                           ActionItem::Home => {
                               if let Some(selected) = pod_table_state.selected() {
                                   if selected > 0 {
                                       pod_table_state.select(Some(selected - 1));
                                   } else {
                                       pod_table_state.select(Some(pod_list.len() - 1));
                                   }
                               }
                           }
                       },
                       _ => {}
                   },
                   Event::Tick => {}
               }
        }
            Some(ui_event) = ui_rx.recv() => {
                match ui_event {
                    UIEvent::RefreshPods(pods) => pod_list = pods
                }
            }

           };
    }

    Ok(())
}

impl KubePod {
    pub fn new(pod: &Pod) -> Self {
        let empty = vec![];
        let cs = pod
            .status
            .as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .unwrap_or(&empty);
        let mut r = 0;
        let mut rc = 0;
        for c in cs {
            if c.ready {
                r += 1;
            }

            rc += c.restart_count;
        }

        let empty_str = String::new();
        let phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref())
            .unwrap_or(&empty_str);

        KubePod {
            name: Meta::name(pod),
            ready: r,
            container_count: cs.len() as i32,
            status: phase.to_string(),
            restart_count: rc,
        }
    }
}

fn refresh_pod_list(namespace: &str, mut tx: tokio::sync::mpsc::Sender<UIEvent>) -> Result<()> {
    let n: String = namespace.into();
    tokio::spawn(async move {
        match get_pods(&n).await {
            Ok(l) => {
                let pod_list: Vec<KubePod> = l.iter().map(|p| KubePod::new(p)).collect();

                let _ = tx.send(UIEvent::RefreshPods(pod_list)).await;
            }
            Err(e) => println!("{:?}", e),
        }
    });

    Ok(())
}

fn render_pods<'a>(pod_list: &[KubePod]) -> Table<'a> {
    let rows: Vec<_> = pod_list
        .iter()
        .map(|p| {
            Row::new(vec![
                Cell::from(Span::raw(p.name.to_string())),
                Cell::from(Span::raw(format!("{}/{}", p.ready, p.container_count))),
                Cell::from(Span::raw(format!("{}", p.restart_count))),
                Cell::from(Span::raw(p.status.clone())),
            ])
        })
        .collect();

    let pod_detail = Table::new(rows)
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
        ])
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );

    pod_detail
}
